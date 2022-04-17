# Central API

The central api for the file-share-platform. Handles websocket conenctions from server agents, and requests from clients who wish to download a file.

## Instructions
```
git clone https://github.com/file-share-platform/central-api.git
cd central-api
touch agents.db
export DATABASE_URL=./agents.db
cargo run --release
```