# Session Handoff - 2026-03-20

## Recent Changes (this session)

### 1. Native Anthropic API Support
- **File**: `crates/river-gateway/src/loop/model.rs`
- Added `Provider` enum (OpenAI, Anthropic) with auto-detection based on URL
- Anthropic Messages API implementation with proper request/response handling
- Ephemeral caching: `cache_control` on first system prompt + last non-tool message
- Logs cache stats (cache_creation_tokens, cache_read_tokens)
- Uses `x-api-key` header, reads `ANTHROPIC_API_KEY` env var

### 2. Nix Configuration Updates
- **Files**: `nix/lib.nix`, `nix/home-module.nix`, `nix/nixos-module.nix`
- Added `services.river.anthropic` options:
  - `enable`, `apiKeyFile`, `baseUrl`, `model`
- Added `environmentFile` option for agents (simpler than per-provider keys)
- Home-manager services now source user's profile for PATH (shell commands work)
- NixOS system services use explicit PATH with bash, coreutils, curl, pandoc, ddgr

### 3. Other Fixes
- Changed wake trigger messages from system to user role (API requirement)
- Heartbeat message simplified to `:heartbeat:`
- Added `--context-limit` CLI argument
- Fixed snowflake generator thread safety (mutex-based approach)
- Version bumped to 0.1.4

## Current Config Structure

```nix
services.river = {
  package.src = /home/cassie/river-engine;

  anthropic = {
    enable = true;
    apiKeyFile = /path/to/key;  # Just the raw key, no KEY= prefix
    model = "claude-3-5-haiku-20241022";  # Use full model ID
  };

  agents.NAME = {
    enable = true;
    workspace = "/path/to/workspace";
    dataDir = "/path/to/data";
    environmentFile = /path/to/env;  # Optional: KEY=value format
    # ... other options

    discord.enable = true;  # If you want discord adapter
    discord.tokenFile = /path/to/token;
    discord.guildId = 123456789;
    discord.port = 3002;
  };
};
```

## Known Issues

### Tool Test Report (from agent)
- **bash**: Was failing with "No such file or directory" - fixed by sourcing user profile
- **webfetch**: Needs curl in PATH - should be fixed now
- **websearch**: Needs ddgr in PATH - should be fixed now
- **Discord service**: May not be starting - check `discord.enable = true` in agent config

### To Debug
```bash
# Check service status
systemctl --user status river-NAME-gateway
systemctl --user status river-NAME-discord

# Check logs
journalctl --user -u river-NAME-gateway -f

# Check if discord service exists
systemctl --user list-units '*river*'
```

## Files Changed
```
Cargo.toml                                   - version 0.1.4
crates/river-core/src/snowflake/generator.rs - mutex-based thread safety
crates/river-gateway/src/loop/context.rs     - wake triggers as user messages
crates/river-gateway/src/loop/mod.rs         - heartbeat as user message
crates/river-gateway/src/loop/model.rs       - Anthropic provider + caching
crates/river-gateway/src/main.rs             - context-limit arg
crates/river-gateway/src/server.rs           - context_limit config
nix/home-module.nix                          - anthropic, environmentFile, PATH
nix/lib.nix                                  - anthropic options
nix/nixos-module.nix                         - anthropic, environmentFile, PATH
nix/packages.nix                             - version 0.1.4
```

## Recent Commits
```
152707a fix(nix): source user profile for PATH in home-manager services
8c7f58e fix(nix): add PATH to service environment for shell commands
47574c8 fix(gateway): restore cache_control on last message for better cache hits
096042f feat(gateway): add native Anthropic API support with ephemeral caching
```
