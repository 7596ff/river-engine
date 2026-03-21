# Shell Profile Loading

**Status:** Draft
**Author:** Cassie
**Date:** 2026-03-21

## Problem

The bash tool runs commands without the user's shell environment:

```rust
Command::new("bash")
    .arg("-c")
    .arg(&command)
```

This is a non-login, non-interactive shell. It doesn't source:
- `~/.bash_profile` / `~/.profile`
- `~/.bashrc`
- User PATH additions (nvm, pyenv, cargo, etc.)

Additionally, the Nix service sets a hardcoded limited PATH:
```nix
defaultPath = lib.makeBinPath (with pkgs; [ bash coreutils curl pandoc ddgr git ]);
```

**Result:** Commands like `node`, `python`, `cargo` may not be found.

## Solution

Run commands in a login shell that sources the user's profile.

### Option A: Login Shell (Recommended)

```rust
Command::new("bash")
    .arg("-l")  // Login shell
    .arg("-c")
    .arg(&command)
```

A login shell sources (in order):
1. `/etc/profile`
2. `~/.bash_profile` OR `~/.bash_login` OR `~/.profile`

### Option B: Interactive Shell

```rust
Command::new("bash")
    .arg("-i")  // Interactive shell
    .arg("-c")
    .arg(&command)
```

An interactive shell sources:
1. `~/.bashrc`

### Option C: Explicit Source

```rust
let wrapped_command = format!("source ~/.bashrc 2>/dev/null; {}", command);
Command::new("bash")
    .arg("-c")
    .arg(&wrapped_command)
```

### Recommendation

Use **Option A (login shell)** because:
- Most consistent with user expectations
- Sources the same profile as terminal login
- Works with most environment managers (nvm, pyenv, etc.)

## Implementation

### 1. Update BashTool

```rust
// src/tools/shell.rs line 88-92

let child = tokio::process::Command::new("bash")
    .arg("-l")  // ADD: login shell
    .arg("-c")
    .arg(&command)
    .current_dir(&workspace)
    .output();
```

### 2. Update Nix Service (Optional Enhancement)

Source the user profile before starting the service:

```nix
# nix/home-module.nix
Service = commonServiceConfig // {
  ExecStart = gatewayCmd;
  Environment = [
    "HOME=%h"
    # Don't override PATH - let shell profile set it
  ];
};
```

Or wrap the command:

```nix
ExecStart = pkgs.writeShellScript "river-gateway-wrapper" ''
  source ~/.profile 2>/dev/null || true
  source ~/.bashrc 2>/dev/null || true
  exec ${gatewayCmd}
'';
```

### 3. Configuration Option (Future)

Allow users to control this behavior:

```toml
# PREFERENCES.toml
[shell]
login_shell = true  # Use -l flag
source_profile = "~/.bashrc"  # Explicit profile to source
```

## Testing

```rust
#[tokio::test]
async fn test_bash_sources_profile() {
    // Create a temp profile that sets a custom variable
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(".bashrc"), "export TEST_VAR=hello").unwrap();

    let tool = BashTool::new(dir.path());
    let result = tool.execute(json!({
        "command": "echo $TEST_VAR"
    })).unwrap();

    assert!(result.output.contains("hello"));
}
```

## Files to Modify

| File | Change |
|------|--------|
| `src/tools/shell.rs` | Add `-l` flag to bash command |
| `nix/home-module.nix` | (Optional) Remove hardcoded PATH |

## Caveats

1. **Performance:** Login shells are slightly slower (profile sourcing)
2. **Side effects:** Profiles might print output, set unexpected variables
3. **Non-bash users:** This assumes bash; zsh/fish users need different handling

## Future: Shell Detection

Detect user's preferred shell:

```rust
let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

if shell.ends_with("zsh") {
    Command::new("zsh").arg("-l").arg("-c").arg(&command)
} else if shell.ends_with("fish") {
    Command::new("fish").arg("-l").arg("-c").arg(&command)
} else {
    Command::new("bash").arg("-l").arg("-c").arg(&command)
}
```

## Summary

Change one line:

```diff
- .arg("-c")
+ .arg("-l")
+ .arg("-c")
```

This gives the agent access to the user's full shell environment.
