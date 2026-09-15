# Install

## From crates.io

```sh
cargo install layover-cli
```

The binary is called `layover`, not `layover-cli`.

```sh
layover --version
```

## From source

```sh
git clone https://github.com/KotkaZ/layover-project
cd layover-project
cargo install --path crates/layover-cli
```

## Agent CLIs

Layover supervises other tools; it does not replace them. Install whichever runners your factory
names, and make sure each works on its own before pointing Layover at it:

| Runner | Install | Check |
|---|---|---|
| Claude Code | `npm i -g @anthropic-ai/claude-code` | `claude --version` |
| GitHub Copilot CLI | `npm i -g @github/copilot` | `copilot --version` |
| OpenAI Codex CLI | `npm i -g @openai/codex` | `codex --version` |

Credentials reach child CLIs through the environment. **Never put an API key in `layover.toml`** —
it is a file people commit.

## Verify your setup

```sh
layover validate --config layover.toml --strict
```

This exits non-zero if anything would stop the factory starting. It is worth running in CI over
your factory definition: an unattended factory that discovers a typo three agents deep has
already spent money to find out.
