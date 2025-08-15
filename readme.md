# Surf - Modern HTTP Command-Line Client

![Rust](https://img.shields.io/badge/Rust-1.70%2B-blue) ![License](https://img.shields.io/badge/License-GPLv3-orange)

Surf is a modern HTTP client with advanced features for developers and system administrators. It provides intuitive command-line tools for interacting with web services, downloading files, benchmarking web endpoints, and managing configurations across different environments.

## Features

- 🚀 **Fetch URLs** with detailed response inspection and analysis
- ⬇️ **Download files** with progress display and resumable transfers
- 📊 **Benchmark URLs** with customizable request parameters and detailed metrics
- 🔧 **Configuration management** with global settings and environment profiles
- 📚 **Request history** tracking with search and replay capabilities
- 👤 **Profile system** for managing different API environments
- 🔍 **Response analysis** with security headers and performance insights
- 🎨 **JSON formatting** with syntax highlighting
- 🔒 Support for custom headers and authentication
- 📄 Comprehensive logging system
- 🌐 Experimental HTTP/3 support
- 🔄 Automatic redirect following
- 📈 Verbose output with performance metrics

## Installation

### Prerequisites
- Rust 1.70+ and Cargo

### Install from source
```bash
cargo install --git https://github.com/Daihongyi/surf.git
```

### Or enable HTTP/3 support
```bash
RUSTFLAGS='--cfg reqwest_unstable' cargo install --git https://github.com/Daihongyi/surf.git --features http3
```

### Build from source
```bash
git clone https://github.com/Daihongyi/surf.git
cd surf
cargo build --release
```

### Enable HTTP/3 (experimental)
```bash
RUSTFLAGS='--cfg reqwest_unstable' cargo build --features http3
```
The binary will be available at `target/release/surf`.

## Usage

### Global Options
Available for all commands:
- `--log`: Enable logging to file
- `--profile <NAME>`: Use a specific configuration profile
- `--no-color`: Disable colored output

### Basic structure
```bash
surf [GLOBAL_OPTIONS] [COMMAND] [OPTIONS]
```

## Commands

### 1. Fetch a URL (`get`)
Fetch a URL and display the response with optional headers and analysis.

```bash
surf get [OPTIONS] <URL>
```

**Options:**
- `-i`, `--include`: Include response headers in output
- `-o`, `--output <FILE>`: Save output to file
- `-L`, `--location`: Follow redirects
- `-H`, `--headers <HEADER>`: Set custom headers (e.g., `Authorization: Bearer token`)
- `-t`, `--connect-timeout <SECONDS>`: Connection timeout (default: 10)
- `-v`, `--verbose`: Display verbose output
- `--http3`: Use HTTP/3 (experimental)
- `--json`: Pretty print JSON responses
- `--analyze`: Analyze response headers for security and performance
- `--save-history <BOOL>`: Save request to history (default: true)

**Examples:**
```bash
# Basic GET request
surf get https://api.github.com/users/octocat

# With headers, JSON formatting, and analysis
surf get -H "Accept: application/json" --json --analyze https://api.github.com/users/octocat

# Using a profile for API development
surf --profile dev get /users/me

# Save to file with verbose output and logging
surf --log get -o response.json -v https://api.example.com/data
```

### 2. Download files (`download`)
Download files with progress display, parallel connections, and resumable transfers.

```bash
surf download [OPTIONS] <URL> <OUTPUT>
```

**Options:**
- `-p`, `--parallel <NUM>`: Number of parallel connections (default: 4)
- `-c`, `--continue-download`: Continue interrupted download
- `-t`, `--idle-timeout <SECONDS>`: Idle timeout between packets (default: 30)
- `--http3`: Use HTTP/3 (experimental)

**Examples:**
```bash
# Basic download
surf download https://example.com/file.zip output.zip

# Resume interrupted download with 8 parallel connections
surf download -c -p 8 https://example.com/large-file.iso output.iso

# Download with logging enabled
surf --log download https://cdn.example.com/software.tar.gz software.tar.gz
```

### 3. Benchmark URLs (`bench`)
Benchmark a URL by sending multiple concurrent requests with detailed performance analysis.

```bash
surf bench [OPTIONS] <URL>
```

**Options:**
- `-n`, `--requests <NUM>`: Number of requests to send (default: 100)
- `-c`, `--concurrency <NUM>`: Number of concurrent connections (default: 10)
- `-t`, `--connect-timeout <SECONDS>`: Connection timeout (default: 5)
- `--http3`: Use HTTP/3 (experimental)

**Examples:**
```bash
# Basic benchmark
surf bench https://api.example.com/health

# Heavy load test with detailed logging
surf --log bench -n 1000 -c 50 https://api.example.com/endpoint

# Using profile for environment-specific testing
surf --profile prod bench /api/status -n 500 -c 25
```

### 4. Configuration Management (`config`)
Manage global application settings.

```bash
surf config <ACTION>
```

**Actions:**
- `show`: Display current configuration
- `reset`: Reset configuration to defaults
- `set <KEY> <VALUE>`: Set a configuration value

**Supported configuration keys:**
- `timeout`: Default connection timeout (seconds)
- `user_agent`: Default User-Agent string
- `max_redirects`: Maximum number of redirects to follow

**Examples:**
```bash
# View current configuration
surf config show

# Set default timeout to 45 seconds
surf config set timeout 45

# Set custom User-Agent
surf config set user_agent "MyApp/1.0 (Testing)"

# Reset all settings to defaults
surf config reset
```

### 5. Request History (`history`)
Track, search, and replay previous requests.

```bash
surf history <ACTION>
```

**Actions:**
- `list [-n NUM]`: Show recent requests (default: 10)
- `search <QUERY>`: Search history by URL, method, or error message
- `show <ID>`: Display detailed information for a specific request
- `clear`: Clear all history

**Examples:**
```bash
# Show last 20 requests
surf history list -n 20

# Search for GitHub API requests
surf history search github

# Show detailed info for a specific request (use first 8 chars of ID)
surf history show 12345678

# Clear all history
surf history clear
```

### 6. Profile Management (`profile`)
Create and manage configuration profiles for different environments or APIs.

```bash
surf profile <ACTION>
```

**Actions:**
- `list`: List all available profiles
- `create <NAME> [OPTIONS]`: Create or update a profile
- `delete <NAME>`: Delete a profile
- `show <NAME>`: Show profile details

**Profile creation options:**
- `--base-url <URL>`: Set base URL for the profile
- `--timeout <SECONDS>`: Override default timeout
- `--follow-redirects`: Enable redirect following

**Examples:**
```bash
# Create development environment profile
surf profile create dev --base-url https://api-dev.company.com --timeout 30 --follow-redirects

# Create production profile with strict settings
surf profile create prod --base-url https://api.company.com --timeout 10

# List all profiles
surf profile list

# Use a profile
surf --profile dev get /users/me

# Show profile details
surf profile show dev

# Delete old profile
surf profile delete old-config
```

## Output Examples

### Verbose GET request with analysis
```
> HTTP/1.1 200 OK
> content-type: application/json; charset=UTF-8
> content-length: 1256
> x-ratelimit-remaining: 4999
> 
{
  "login": "octocat",
  "id": 1,
  "name": "The Octocat"
}

=== Response Analysis ===
security.strict-transport-security: missing
security.content-security-policy: present
security.x-frame-options: present
server.type: GitHub.com
cache.control: private, max-age=60
=== End Analysis ===

< Status: 200 | Size: 1.23 KB | Time: 145ms | Server: GitHub.com
```

### File download with progress
```
[00:00:05] [##############################] 5.12 MB/5.12 MB (1s) | 1.05 MB/s | Downloading...
Downloaded 5.12 MB in 4.87s (avg: 1.05 MB/s) to: /home/user/output.zip
```

### Benchmark results
```
Benchmarking https://api.example.com with 100 requests, concurrency 10 (HTTP/3: false)

=== Benchmark Results ===
Total time: 1.87s
Requests per second: 53.48
Successful requests: 98
Failed requests: 2

Response Times (ms):
  Min: 89
  Max: 324
  Avg: 145
  50th percentile: 142
  95th percentile: 267
  99th percentile: 312

Status Code Distribution:
  200: 98 (98.0%)
  500: 2 (2.0%)
```

### History listing
```
Recent requests:
2025-08-15 14:30:25 | GET https://api.github.com/users/octocat | 200 ✓ | 145ms | a1b2c3d4
2025-08-15 14:28:10 | GET https://api.example.com/health | 200 ✓ | 89ms | e5f6g7h8
2025-08-15 14:25:33 | GET https://slow-api.com/data | Error | N/A | i9j0k1l2
```

## Configuration Files

### Config file location
- **Global config**: `~/.config/surf/config.toml`
- **History**: `~/.local/share/surf/history.json`
- **Logs**: `./surf.log` (current directory) or alongside output files

### Sample config file
```toml
default_timeout = 30
default_user_agent = "surf/0.2.1"
max_redirects = 10

[default_headers]
"User-Agent" = "surf/0.2.1"
"Accept" = "application/json"

[profiles.dev]
name = "dev"
base_url = "https://api-dev.example.com"
timeout = 45
follow_redirects = true

[profiles.prod]
name = "prod"
base_url = "https://api.example.com"
timeout = 15
follow_redirects = false
```

## Advanced Usage Examples

### API Development Workflow
```bash
# 1. Create API profiles for different environments
surf profile create dev --base-url https://api-dev.company.com --follow-redirects
surf profile create prod --base-url https://api.company.com --timeout 10

# 2. Test API endpoints
surf --profile dev get /health --analyze --json

# 3. Run performance tests
surf --profile dev bench /api/heavy-endpoint -n 100 -c 10

# 4. Check request history
surf history search "company.com"
```

### Multi-environment Testing
```bash
# Test the same endpoint across environments
for env in dev staging prod; do
  echo "Testing $env environment:"
  surf --profile $env get /health --json | jq '.status'
done
```

### File Management with Logging
```bash
# Download with detailed logging for debugging
surf --log download https://releases.company.com/v2.1/app.tar.gz ./downloads/app.tar.gz

# Check download logs
tail -f surf.log
```

## Contributing

Contributions are welcome! Please follow these steps:
1. Fork the repository
2. Create a new branch for your feature
3. Commit your changes with descriptive messages
4. Push to your fork and submit a pull request

## License

This project is licensed under the GPLv3 License - see the [LICENSE](LICENSE) file for details.