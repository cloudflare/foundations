# bedrock

A Rust service foundation framework.

## Documentation
You would need Nightly Rust, to install it run:
```
rustup install nightly
```

And then checkout the repo and run:
```
rustup run nightly cargo rustdoc --lib -p bedrock --open --  --cfg docsrs
```

## Releasing

#### Prerequisites

Install `cargo-release` and `git-cliff` on your machine:
```bash
cargo install cargo-release git-cliff
```

To create a release, bumping the patch version:
```
cargo release patch -x --no-push
```

Then, push the tags/commits:
```
cargo release push -x
```
