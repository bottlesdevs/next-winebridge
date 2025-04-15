# next-winebridge
The source code of the Next version of WineBridge

## Testing
Tests for winebridge are meant to be run inside wine. Generate the test executables using the following command:
```bash
cargo test --no-run --lib
```

This will generate the test executables in the `target/$TARGET/debug/deps` which can be run using wine as follows:
```bash
wine target/$TARGET/debug/deps/next-winebridge-*.exe --test-threads=1
```

The `--test-threads=1` flag is required becase parallel operations on Wine registry can interfere with each other and cause tests to fail. This issue is not specific to this project.
