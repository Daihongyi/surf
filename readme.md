
# Surf - Modern HTTP Command-Line Client

![Rust](https://img.shields.io/badge/Rust-1.70%2B-blue) ![License](https://img.shields.io/badge/License-MIT-orange)

Surf is a modern HTTP client with advanced features for developers and system administrators. It provides intuitive command-line tools for interacting with web services, downloading files, and benchmarking web endpoints.

## Features

- 🚀 **Fetch URLs** with detailed response inspection
- ⬇️ **Download files** with progress display and resumable transfers
- 📊 **Benchmark URLs** with customizable request parameters
- 🔒 Support for custom headers and timeouts
- 🔄 Automatic redirect following
- 📈 Verbose output with performance metrics
- 🌐 Experimental HTTP/3 support

## Installation

### Prerequisites
- Rust 1.70+ and Cargo

### Install from source
```bash
cargo install --git https://github.com/Daihongyi/surf.git
```
### Or enable the http3
```bash
RUSTFLAGS='--cfg reqwest_unstable' cargo install --git https://github.com/Daihongyi/surf.git --features http3
```


### Build from source
```bash
git clone https://github.com/Daihongyi/surf.git
cd surf
cargo build --release
```
### Enable http3 (beta)
```bash
RUSTFLAGS='--cfg reqwest_unstable' cargo build --features http3
```
The binary will be available at `target/release/surf`.



## Usage

### Basic structure
```bash
surf [COMMAND] [OPTIONS]
```

### Commands

#### 1. Fetch a URL (`get`)
Fetch a URL and display the response with optional headers.

```bash
surf get [OPTIONS] <URL>
```

**Options:**
- `-i`, `--include`: Include response headers in output
- `-o`, `--output <FILE>`: Save output to file
- `-L`, `--location`: Follow redirects (default: false)
- `-H`, `--headers <HEADER>`: Set custom headers (e.g., `Authorization: Bearer token`)
- `-t`, `--timeout <SECONDS>`: Timeout in seconds (default: 30)
- `-v`, `--verbose`: Display verbose output
- `--http3`: Use HTTP/3 (experimental)

**Examples:**
```bash
# Basic GET request
surf get https://example.com

# With headers and redirects
surf get -H "Accept: application/json" -L https://example.com/redirect

# Save to file with verbose output
surf get -o output.html -v https://example.com
```

#### 2. Download files (`download`)
Download files with progress display and resumable transfers.

```bash
surf download [OPTIONS] <URL> <OUTPUT>
```

**Options:**
- `-p`, `--parallel <NUM>`: Number of parallel connections (default: 4)
- `-c`, `--continue-download`: Continue interrupted download
- `-t`, `--timeout <SECONDS>`: Timeout in seconds (default: 30)

**Examples:**
```bash
# Basic download
surf download https://example.com/file.zip output.zip

# Resume interrupted download
surf download -c https://example.com/large-file.iso output.iso

# Download with 8 parallel connections
surf download -p 8 https://fast.server/bigfile.tar.gz archive.tar.gz
```

#### 3. Benchmark URLs (`bench`)
Benchmark a URL by sending multiple requests.

```bash
surf bench [OPTIONS] <URL>
```

**Options:**
- `-n`, `--requests <NUM>`: Number of requests to send (default: 100)
- `-c`, `--concurrency <NUM>`: Number of concurrent connections (default: 10)
- `-t`, `--timeout <SECONDS>`: Timeout in seconds (default: 5)

**Examples:**
```bash
# Basic benchmark (100 requests, 10 concurrency)
surf bench https://api.example.com

# Heavy load test
surf bench -n 1000 -c 50 https://stress-test.example.com
```

## Output Examples

### Verbose GET request
```
> HTTP/1.1 200 OK
> content-type: text/html; charset=UTF-8
> content-length: 1256
> 
<!DOCTYPE html>
...
< Response size: 1.23 KB
```

### File download
```
[00:00:05] [##############################] 5.12 MB/5.12 MB (1s) | 1.05 MB/s
Downloaded 5.12 MB in 4.87s (avg: 1.05 MB/s) to: /path/to/output.zip
```

### Benchmark results
```
Benchmarking https://example.com with 100 requests, concurrency 10
...
Status: 200 | Time: 145.23ms
Status: 200 | Time: 152.81ms

Benchmark complete
Total time: 1.87s
Requests per second: 53.48
```

## Contributing

Contributions are welcome! Please follow these steps:
1. Fork the repository
2. Create a new branch for your feature
3. Commit your changes with descriptive messages
4. Push to your fork and submit a pull request

## License

This project is licensed under the GPLv3 License - see the [LICENSE](LICENSE) file for details.
```