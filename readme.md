# Central API

The central api for the file-share-platform. Handles websocket conenctions from server agents, and requests from clients
who wish to download a file.

## Instructions
```
git clone https://github.com/file-share-platform/central-api.git
cd central-api
cargo run
```
Note that when running in debug it will use reasonable defaults for the enviromental variables.
However, if run with `cargo run --release` you must provide these enviromental variables yourself.