# tusc

A [tus](https://tus.io) **resumable upload client** for Rust.

> **Work in progress.** The API is not stable yet.

## What is tus?

[tus](https://tus.io/protocols/resumable-upload) is an open protocol for resumable file uploads over HTTP. Uploads can be interrupted and resumed without re-uploading data, making it ideal for large files and unreliable networks.

## Features

| Feature                        | Status                            |
|--------------------------------|-----------------------------------|
| Core protocol (HEAD, PATCH)    | ✅ Done                            |
| Creation extension (POST)      | ✅ Done                            |
| Termination extension (DELETE) | ✅ Done                            |
| Creation with upload           | 🚧 Planned                        |
| Expiration handling            | 🚧 Partial (parsed, not acted on) |
| Chunked uploads                | ✅ Done                            |
| Checksum extension             | ❌ Not planned                     |
| Concatenation extension        | ❌ Not planned                     |

## Usage

```rust
// Coming soon
```

## Protocol compliance

**tusc** targets [tus v1.0.0](https://tus.io/protocols/resumable-upload) and does not depend on a specific HTTP version.

## License

This project is licensed under the [MIT License](LICENSE).