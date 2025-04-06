# stream-start-screen

This is a personal fork of the `stream-start-screen` project.

## Description

`stream-start-screen` is a Rust application that displays a customizable pre-stream starting screen using OpenGL (`glow`) and `glfw` for windowing. It renders a 3D scene containing a monitor, screen, table, and walls. The screen displays dynamic text including:

- The stream topic for the day.
- The scheduled stream start time.
- The current time.
- A countdown timer indicating the time remaining until the stream starts.

## Development

Ensure you have Rust, Cargo, and Cmake installed.

To run the application, use the following command:

```bash
cargo run -- --start-time HH:MM:SS --topic "Your Stream Topic"
```

## Building

To build the project, ensure you have Rust and Cargo installed. Then run:

```bash
cargo build --release
```

The executable will be located at `target/release/stream-start-screen`.

## Usage

The application requires two command-line arguments:

```bash
./target/release/stream-start-screen --start-time HH:MM:SS --topic "Your Stream Topic"
```

Replace `HH:MM:SS` with the stream start time (24-hour format) and `"Your Stream Topic"` with the relevant topic.

Example:

```bash
./target/release/stream-start-screen --start-time 14:00:00 --topic "Working on a Rust project"
```

## Troubleshooting

For Mac users, you may encounter this [issue](https://www.reddit.com/r/opengl/comments/14oazju/version_330_is_not_supported_m1_mac/). I fixed it in [main.rs](src/main.rs#L500-L505).
