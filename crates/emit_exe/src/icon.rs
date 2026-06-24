//! `.ico` parsing and `RT_GROUP_ICON` / `RT_ICON` resource assembly.
//!
//! Ports the byte-level work of Ahk2Exe's `IconChanger.ahk` (`AddOrReplaceIcon`). A `.ico` file
//! is an `ICONDIR` header followed by N 16-byte `ICONDIRENTRY` records, each ending in a 4-byte
//! file offset to that image's bytes. Inside a PE the same directory becomes an `RT_GROUP_ICON`
//! resource whose entries are 14 bytes - the leading 12 bytes copied verbatim, the trailing
//! 4-byte file offset replaced by a 2-byte `RT_ICON` resource id - and each image's bytes live in
//! its own `RT_ICON` resource under that id.
//!
//! This module is pure (no Win32 / PE I/O) so it is unit-testable on any platform; the caller
//! supplies the interpreter's existing primary-group member ids (to reuse) and the highest icon id
//! in use (to mint fresh ids above), and writes the returned resources via `UpdateResource`.

use anyhow::{bail, Result};

/// Size of an on-disk `.ico` directory entry (`ICONDIRENTRY`).
const FILE_ENTRY_LEN: usize = 16;
/// Size of an in-PE group-icon directory entry (`GRPICONDIRENTRY`): 12 shared bytes + 2-byte id.
const GROUP_ENTRY_LEN: usize = 14;
/// Size of the `ICONDIR` / `GRPICONDIR` header.
const DIR_HEADER_LEN: usize = 6;

/// One icon image parsed from a `.ico`: the 12 shared directory bytes (`width`, `height`,
/// `colors`, `reserved`, `planes`, `bit_count`, `bytes_in_res`) and the raw image payload.
struct IconImage {
    meta: [u8; 12],
    data: Vec<u8>,
}

/// The resources to inject for a primary-icon replacement.
pub struct IconResources {
    /// `RT_GROUP_ICON` id to (over)write - the interpreter's primary group.
    pub group_id: u16,
    /// Rebuilt `RT_GROUP_ICON` directory bytes.
    pub group_data: Vec<u8>,
    /// `(RT_ICON id, image bytes)` for each image in the new icon.
    pub images: Vec<(u16, Vec<u8>)>,
    /// Old `RT_ICON` member ids of the primary group that the new icon does not reuse - delete
    /// them so no orphaned images are left behind (matching Ahk2Exe).
    pub stale_image_ids: Vec<u16>,
}

/// Mints fresh `RT_ICON` image ids strictly above every id already in the PE, so a new icon's
/// images never clobber a built-in (tray/Gui) image. One allocator is shared across the primary
/// icon and all additional icons in a single bundle so their minted ids never collide.
pub struct IconIdAllocator {
    next: u16,
}

impl IconIdAllocator {
    /// Start minting strictly above `max_existing` (the highest `RT_ICON` id already in the PE).
    pub fn new(max_existing: u16) -> Self {
        Self { next: max_existing }
    }

    fn mint(&mut self) -> u16 {
        self.next = self.next.checked_add(1).expect("icon id overflow");
        self.next
    }
}

/// Build the `RT_GROUP_ICON` + `RT_ICON` resources for `ico_bytes` under group id `group_id`.
/// Member ids are reused from `existing_member_ids` where possible (so replacing the primary group
/// recycles its image ids); any extra images get fresh ids from `alloc`. Pass an empty
/// `existing_member_ids` to file a brand-new group whose images are all freshly minted.
pub fn build_icon_resources(
    ico_bytes: &[u8],
    group_id: u16,
    existing_member_ids: &[u16],
    alloc: &mut IconIdAllocator,
) -> Result<IconResources> {
    let images = parse_ico(ico_bytes)?;

    // Assign an RT_ICON id per image: reuse the old group's member ids first, then mint new ones.
    let ids: Vec<u16> = (0..images.len())
        .map(|i| match existing_member_ids.get(i) {
            Some(&id) => id,
            None => alloc.mint(),
        })
        .collect();

    // Rebuild the group directory: 6-byte header (reserved=0, type=1, count=N) then a 14-byte
    // entry per image (12 shared bytes + 2-byte id).
    let mut group_data = Vec::with_capacity(DIR_HEADER_LEN + images.len() * GROUP_ENTRY_LEN);
    group_data.extend_from_slice(&0u16.to_le_bytes()); // reserved
    group_data.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    group_data.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (img, &id) in images.iter().zip(&ids) {
        group_data.extend_from_slice(&img.meta);
        group_data.extend_from_slice(&id.to_le_bytes());
    }

    let resource_images = images
        .into_iter()
        .zip(&ids)
        .map(|(img, &id)| (id, img.data))
        .collect();

    let stale_image_ids = existing_member_ids
        .iter()
        .skip(ids.len().min(existing_member_ids.len()))
        .copied()
        .collect();

    Ok(IconResources {
        group_id,
        group_data,
        images: resource_images,
        stale_image_ids,
    })
}

/// Parse a `.ico` into its images. Validates the directory header and each entry's bounds.
fn parse_ico(bytes: &[u8]) -> Result<Vec<IconImage>> {
    if bytes.len() < DIR_HEADER_LEN {
        bail!("icon file is too small to contain a directory header");
    }
    let type_ = u16::from_le_bytes([bytes[2], bytes[3]]);
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if type_ != 1 {
        bail!("not an icon file (ICONDIR type is {type_}, expected 1)");
    }
    if count == 0 {
        bail!("icon file contains no images");
    }

    let mut images = Vec::with_capacity(count);
    for i in 0..count {
        let off = DIR_HEADER_LEN + i * FILE_ENTRY_LEN;
        let entry = bytes
            .get(off..off + FILE_ENTRY_LEN)
            .ok_or_else(|| anyhow::anyhow!("icon directory entry {i} is out of bounds"))?;
        let bytes_in_res = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
        let image_offset =
            u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
        let data = bytes
            .get(image_offset..image_offset + bytes_in_res)
            .ok_or_else(|| anyhow::anyhow!("icon image {i} bytes are out of bounds"))?
            .to_vec();
        let mut meta = [0u8; 12];
        meta.copy_from_slice(&entry[0..12]);
        images.push(IconImage { meta, data });
    }
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid `.ico` with `n` 1-byte "images".
    fn fake_ico(n: u16) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&0u16.to_le_bytes()); // reserved
        v.extend_from_slice(&1u16.to_le_bytes()); // type
        v.extend_from_slice(&n.to_le_bytes()); // count
        let data_start = DIR_HEADER_LEN + n as usize * FILE_ENTRY_LEN;
        for i in 0..n {
            // 12 meta bytes - width/height/etc. encode the index so we can assert preservation.
            let meta = [i as u8, 0, 0, 0, 1, 0, 8, 0, 1, 0, 0, 0]; // bytes_in_res = 1
            v.extend_from_slice(&meta);
            let offset = (data_start + i as usize) as u32;
            v.extend_from_slice(&offset.to_le_bytes());
        }
        for i in 0..n {
            v.push(0xA0 + i as u8); // one payload byte per image
        }
        v
    }

    #[test]
    fn reuses_existing_ids_and_substitutes_offsets() {
        let ico = fake_ico(2);
        let res = build_icon_resources(&ico, 1, &[5, 6], &mut IconIdAllocator::new(99)).unwrap();
        assert_eq!(res.group_id, 1);
        // group header: reserved, type=1, count=2
        assert_eq!(&res.group_data[0..6], &[0, 0, 1, 0, 2, 0]);
        // first entry: 12 meta bytes then id 5
        assert_eq!(res.group_data[6], 0); // meta[0] == image index 0
        assert_eq!(
            u16::from_le_bytes([res.group_data[18], res.group_data[19]]),
            5
        );
        // second entry id 6
        assert_eq!(res.group_data[6 + GROUP_ENTRY_LEN], 1); // meta[0] == image index 1
        assert_eq!(
            u16::from_le_bytes([
                res.group_data[6 + GROUP_ENTRY_LEN + 12],
                res.group_data[6 + GROUP_ENTRY_LEN + 13]
            ]),
            6
        );
        assert_eq!(res.images, vec![(5, vec![0xA0]), (6, vec![0xA1])]);
        assert!(res.stale_image_ids.is_empty());
    }

    #[test]
    fn mints_fresh_ids_when_new_icon_has_more_images() {
        let ico = fake_ico(3);
        let res = build_icon_resources(&ico, 1, &[5], &mut IconIdAllocator::new(99)).unwrap();
        // first reuses 5, next two are minted above max_icon_id 99
        assert_eq!(
            res.images.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![5, 100, 101]
        );
        assert!(res.stale_image_ids.is_empty());
    }

    #[test]
    fn reports_stale_ids_when_new_icon_has_fewer_images() {
        let ico = fake_ico(1);
        let res = build_icon_resources(&ico, 1, &[5, 6, 7], &mut IconIdAllocator::new(99)).unwrap();
        assert_eq!(res.images, vec![(5, vec![0xA0])]);
        assert_eq!(res.stale_image_ids, vec![6, 7]);
    }

    #[test]
    fn new_group_mints_all_fresh_ids_and_shares_allocator() {
        // An additional icon (no existing members) mints every image id from the shared allocator;
        // a second additional icon continues above the first, so ids never collide.
        let mut alloc = IconIdAllocator::new(27);
        let a = build_icon_resources(&fake_ico(2), 300, &[], &mut alloc).unwrap();
        let b = build_icon_resources(&fake_ico(2), 301, &[], &mut alloc).unwrap();
        assert_eq!(a.group_id, 300);
        assert_eq!(a.images.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [28, 29]);
        assert!(a.stale_image_ids.is_empty());
        assert_eq!(b.group_id, 301);
        assert_eq!(b.images.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [30, 31]);
    }

    #[test]
    fn rejects_non_icon() {
        let mut bad = fake_ico(1);
        bad[2] = 2; // type = cursor
        assert!(build_icon_resources(&bad, 1, &[], &mut IconIdAllocator::new(0)).is_err());
    }
}
