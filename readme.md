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
- 💾 **Configuration caching** for quick command reuse and automation
- 🔐 Support for custom headers and authentication
- 📝 Comprehensive logging system
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
- `-x`, `--use-cache`: Use cached configuration from last run
- `--no-save`: Do not save configuration to cache

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

# With headers, JSON formatting, and analysis (saves to cache)
surf get -H "Accept: application/json" --json --analyze https://api.github.com/users/octocat

# Reuse previous configuration for different URL
surf get https://api.github.com/users/torvalds -x

# Using a profile for API development without saving to cache
surf --profile dev get /users/me --no-save

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
# Basic download (configuration is cached)
surf download https://example.com/file.zip output.zip

# Resume interrupted download with 8 parallel connections
surf download -c -p 8 https://example.com/large-file.iso output.iso

# Use cached configuration for another download
surf download https://cdn.example.com/another-file.tar.gz another-file.tar.gz -x

# Download without saving configuration to cache
surf download --no-save https://temp.com/temp.zip temp.zip -p 2

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
# Basic benchmark (saves configuration)
surf bench https://api.example.com/health

# Heavy load test with same settings as before
surf bench https://api.example.com/endpoint -x

# Override cached concurrency setting (will show conflict error)
surf bench https://api.example.com/test -x -c 20

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

### 7. Cache Management (`cache`) - NEW 🔥
Manage configuration caching for rapid command reuse and automation workflows.

```bash
surf cache <ACTION>
```

**Actions:**
- `show`: Display current cached configuration
- `clear`: Clear all cached configuration

**Key Features:**
- **Automatic Caching**: All successful command executions automatically save their configuration
- **Smart Reuse**: Use `-x` to reuse the exact configuration from your last command
- **Conflict Detection**: Prevents conflicting parameters when using cache
- **Intelligent Merging**: Combines new parameters with cached ones when possible
- **Selective Saving**: Use `--no-save` to prevent caching specific commands

**Examples:**
```bash
# View current cached configuration
surf cache show

# Clear all cached settings
surf cache clear

# Example workflow:
# 1. Run command with specific settings (automatically cached)
surf download https://example.com/file1.zip file1.zip -p 8 --http3

# 2. Reuse exact same settings for different file
surf download https://example.com/file2.zip file2.zip -x

# 3. Check what's cached
surf cache show

# 4. Clear cache when switching contexts
surf cache clear
```

## Cache Usage Patterns

### Basic Caching Workflow
```bash
# Step 1: Execute command with desired parameters (auto-cached)
surf get -H "Authorization: Bearer token" --json --analyze https://api.example.com/users

# Step 2: Reuse configuration for different endpoint
surf get https://api.example.com/posts -x
# Automatically uses: -H "Authorization: Bearer token" --json --analyze

# Step 3: View cached configuration
surf cache show
```

### Download Automation
```bash
# Set up download preferences
surf download https://cdn.example.com/file1.tar.gz file1.tar.gz -p 8 -c --http3

# Download multiple files with same settings
surf download https://cdn.example.com/file2.tar.gz file2.tar.gz -x
surf download https://cdn.example.com/file3.tar.gz file3.tar.gz -x
```

### Conflict Resolution
```bash
# First command caches parallel=4
surf download https://example.com/file1.zip file1.zip -p 4

# This will show conflict error because cached parallel=4 but provided parallel=8
surf download https://example.com/file2.zip file2.zip -x -p 8
# Error: Configuration conflicts detected when using cache:
#   - parallel: cached=4, provided=8

# Use without -x to override cache
surf download https://example.com/file2.zip file2.zip -p 8
```

### Parameter Merging
```bash
# Cache basic settings
surf get https://api.example.com/endpoint --json

# Add new parameter to cached settings
surf get https://api.example.com/other -x --analyze
# Now cache contains both --json and --analyze
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

### Cache configuration display
```bash
surf cache show
```
```
Cached configuration:
  parallel: 8
  continue_download: true
  idle_timeout: 30s
  http3: true
  include: true
  verbose: true
  json: true
  analyze: true
  headers: ["Authorization: Bearer token", "Accept: application/json"]
  connect_timeout: 15s
  no_color: false
  profile: dev
```

## Configuration Files

### Config file locations
- **Global config**: `~/.config/surf/config.toml`
- **History**: `~/.local/share/surf/history.json`
- **Cache**: `~/.config/surf/last_config.json` ⭐ NEW
- **Logs**: `./surf.log` (current directory) or alongside output files

### Sample config file
```toml
default_timeout = 30
default_user_agent = "surf/0.3.0"
max_redirects = 10

[default_headers]
"User-Agent" = "surf/0.3.0"
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

### Sample cache file
```json
{
  "parallel": 8,
  "continue_download": true,
  "idle_timeout": 30,
  "http3": true,
  "include": true,
  "location": false,
  "headers": [
    "Authorization: Bearer token",
    "Accept: application/json"
  ],
  "connect_timeout": 15,
  "verbose": true,
  "json": true,
  "analyze": true,
  "save_history": true,
  "requests": 500,
  "concurrency": 20,
  "no_color": false,
  "profile": "dev"
}
```

## Advanced Usage Examples

### API Development Workflow with Caching
```bash
# 1. Create API profiles for different environments
surf profile create dev --base-url https://api-dev.company.com --follow-redirects
surf profile create prod --base-url https://api.company.com --timeout 10

# 2. Test API endpoint with auth and caching
surf --profile dev get /health --analyze --json -H "Authorization: Bearer dev-token"

# 3. Test multiple endpoints with same configuration
surf --profile dev get /users -x
surf --profile dev get /posts -x
surf --profile dev get /settings -x

# 4. Run performance tests with cached settings
surf --profile dev bench /api/heavy-endpoint -x

# 5. Check cache and history
surf cache show
surf history search "company.com"
```

### Multi-environment Testing with Cache
```bash
# Test dev environment (caches configuration)
surf --profile dev get /health --json --analyze

# Switch to prod (cache is preserved per environment)
surf --profile prod get /health -x  # Uses cached settings for prod profile

# Compare response times across environments
for env in dev staging prod; do
  echo "Testing $env environment:"
  surf --profile $env bench /health -x -n 50
done
```

### File Management Automation
```bash
# Set up download preferences once
surf download https://releases.company.com/v2.1/app.tar.gz ./downloads/app.tar.gz -p 8 --http3

# Batch download with same settings
for version in v2.2 v2.3 v2.4; do
  surf download https://releases.company.com/$version/app.tar.gz ./downloads/app-$version.tar.gz -x
done

# Check download logs
tail -f surf.log
```

### CI/CD Integration Examples
```bash
#!/bin/bash
# deployment-test.sh

# Cache baseline testing configuration
surf --profile staging bench /api/health -n 100 -c 10 --json

# Test all critical endpoints with same configuration
endpoints=("/api/users" "/api/posts" "/api/auth" "/api/upload")
for endpoint in "${endpoints[@]}"; do
  echo "Testing $endpoint..."
  surf --profile staging bench "$endpoint" -x
  if [ $? -ne 0 ]; then
    echo "Benchmark failed for $endpoint"
    exit 1
  fi
done

echo "All benchmarks passed!"
```

### Development Workflow Optimization
```bash
# Morning routine: set up development session
surf --profile dev get /api/auth/me -H "Authorization: Bearer $(cat ~/.dev-token)" --json --analyze

# Throughout the day: quickly test different endpoints
surf get /api/users/123 -x
surf get /api/posts/latest -x  
surf get /api/notifications -x

# End of day: clear cache for fresh start tomorrow
surf cache clear
```

## Cache Behavior Details

### What Gets Cached
- All command-line options and flags (except URLs and file paths)
- Custom headers
- Timeout settings
- HTTP/3 preferences
- Output formatting options (JSON, verbose, etc.)
- Profile selections

### What Doesn't Get Cached
- URLs and endpoints
- Output file paths
- Temporary session data

### Cache Lifecycle
1. **Auto-save**: Every successful command execution saves its configuration
2. **Conflict detection**: Using `-x` with conflicting parameters shows clear errors
3. **Intelligent merging**: New parameters are merged with cached ones when possible
4. **Manual management**: Use `surf cache show/clear` for inspection and cleanup

### Default Values Reference

| Command | Option | Default | Description |
|---------|---------|---------|-------------|
| download | parallel | 4 | Number of parallel connections |
| download | continue_download | false | Resume interrupted downloads |
| download | idle_timeout | 30 | Seconds between packets |
| get | include | false | Show response headers |
| get | location | false | Follow redirects |
| get | connect_timeout | 10 | Connection timeout seconds |
| get | verbose | false | Verbose output |
| get | json | false | Pretty-print JSON |
| get | analyze | false | Analyze response headers |
| get | save_history | true | Save to request history |
| bench | requests | 100 | Number of requests |
| bench | concurrency | 10 | Concurrent connections |
| bench | connect_timeout | 5 | Connection timeout seconds |
| global | http3 | false | Use HTTP/3 protocol |
| global | no_color | false | Disable colored output |

## Contributing

Contributions are welcome! Please follow these steps:
1. Fork the repository
2. Create a new branch for your feature
3. Commit your changes with descriptive messages
4. Push to your fork and submit a pull request

### Recent Changes (v0.3.0)
- ✨ Added configuration caching system
- 🚀 Introduced `-x`/`--use-cache` for rapid command reuse
- 🔧 Added `--no-save` option for selective caching
- 📊 New `surf cache show/clear` commands
- 🧠 Intelligent conflict detection and parameter merging
- 📚 Enhanced automation capabilities for CI/CD workflows

## License

This project is licensed under the GPLv3 License - see the [LICENSE](LICENSE) file for details.