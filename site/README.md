# ahkbuild docs site

Hugo site (hugo-book theme) for the ahkbuild documentation.

## Setup

Requires **Hugo extended** (the SCSS pipeline needs it).

```shell
# from repo root, one-time: pull in the theme
git submodule add https://github.com/alex-shpak/hugo-book site/themes/hugo-book

# serve locally
cd site
hugo serve
```

The site deploys to GitHub Pages via `.github/workflows/pages.yml` on pushes to
`main` that touch `site/`.

## Syntax highlighting

Code blocks use class-based Chroma highlighting (`noClasses = false` in
`hugo.toml`), styled by `assets/_chroma-light.scss` and `assets/_chroma-dark.scss`.
`assets/_custom.scss` loads the dark palette under
`@media (prefers-color-scheme: dark)`, matching the theme's auto light/dark
switch. Both are hand-maintained but use standard Chroma token classes, so you
can regenerate either from a built-in style:

```shell
hugo gen chromastyles --style=github > assets/_chroma-light.scss
hugo gen chromastyles --style=nord   > assets/_chroma-dark.scss   # drop its @media wrapper
```

## Structure

Content lives in `content/docs/`. The left sidebar is built from section
`_index.md` files, ordered by the `weight` front-matter field. See each
section's `_index.md` for what belongs there and which existing `docs/*.md`
design doc it draws from.
