# Mabinogi Pack Utilities

Pack utilities for Mabinogi.

## Build

Use rust 1.39 or above.

```rust
cargo build --release
```

## Usage

```
USAGE:
    mabi-pack [SUBCOMMAND]

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

SUBCOMMANDS:
    extract    Extract a pack
    help       Prints this message or the help of the given subcommand(s)
    list       Output the file list of a pack
    pack       Create a pack
```

To extract all `.xml` and `.txt` files from a pack:

```
mabi-pack extract -i D:\Mabinogi\package\339_full.pack -o D:\data --filter "\.xml" --filter "\.txt"
```

To pack files with version 400:

```
mabi-pack pack -i D:\mydata -o D:\Mabinogi\package\mypack.pack -k 400
```

To list all files with version info:

```
mabi-pack list -i D:\Mabinogi\package\339_full.pack --with-version
```

## License

This program is distributed under the MIT License.