# ghfetch-lite

[crates.io](https://crates.io/crates/ghfetch-lite) · [GitHub](https://github.com/EriksRemess/ghfetch-lite)

Show GitHub profiles in your terminal. Written in Rust with zero Cargo dependencies.
Requires `curl` and Rust 1.85+ to install. Avatars use Kitty image mode by default.

```sh
cargo install ghfetch-lite
ghfetch-lite EriksRemess
ghfetch-lite --help
```

Use `--help` for options or `--no-image` for text only.
Set `GH_TOKEN` or `GITHUB_TOKEN` for higher API rate limits.
