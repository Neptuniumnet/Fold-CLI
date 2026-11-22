# Build FOLD CLI from source

## Requirements

- Linux
- Rust 1.75 or newer (includes Cargo)
- `xdg-utils` to open files with your desktop's default application

## Build and run

```sh
git clone https://github.com/Neptunium-Laboratory/Fold-CLI.git
cd Fold-CLI
cargo build --release
./target/release/foldcli --help
```

Install the binary to Cargo's user bin directory with:

```sh
cargo install --path .
```

Then launch it with `foldcli`. To start in a specific directory, pass it as an argument or use `--path`:

```sh
foldcli --path /home/you/Documents
```

When using `cargo run`, put `--` before arguments passed to FOLD:

```sh
cargo run -- --path /home/you/Documents
```

See [README.md](README.md) for controls, settings, themes, and clipboard notes.
