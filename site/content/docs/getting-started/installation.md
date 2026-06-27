---
title: Installation
weight: 1
---

# Installation

<!-- TODO: set up a release pipeline and allow downloading binaries -->

## Installing with Cargo

`ahkbuild` is a Rust program and can be installed with cargo:

1. [Install Rust](https://rust-lang.org/tools/install/) if you have not already
2. Clone the repository and initialize submodules:

   ```bash
   git clone git@github.com:holy-tao/ahkbuild.git
   cd ahkbuild
   git submodule update --init --recursive
   ```

3. Install with cargo:

   ```bash
   cargo install --path crates/cli
   ```

You can now run `ahkbuild` from the command line.

### Building from source

You can also build from source without installing. After cloning and setting up submodules:

```bash
cargo build --release
./target/release/ahkbuild --help
```

## Verifying the install

Check that the binary is on your `PATH` and prints its help:

```bash
ahkbuild --help
```

> [!IMPORTANT]
> Bundling to a single [`.ahk` file]({{< relref "/docs/bundling" >}}) works on any platform, but
> the [exe target]({{< relref "/docs/exe" >}}) (`ahkbuild bundle exe`) currently **requires
> Windows**. See [PE manipulation]({{< relref "/docs/internals/pe-manipulation" >}}) for why.
