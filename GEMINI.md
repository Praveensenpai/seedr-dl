# Project Rules & Guidelines

- **Gemini CLI & Antigravity Assistant**:
  - Always keep explanations concise and clear.
  - Follow the Rust Codebase Rules specified in `RULES.md` and `.agent/rules/rust.md`.

- **Rust Codebase Rules (`RULES.md`)**:
  - **Hard limits**: <400 lines/file (300 soft), <60 lines/fn (40 soft), max 4 params, max 3 nesting depth.
  - **Zero tolerance**: No `#[allow(dead_code)]`, no `unwrap()`/`expect()` in production code, no bare lint suppressions, zero compiler/clippy warnings (`-D warnings`).
  - **Architecture & Organization**: Role-based modules, bounded responsibilities, clean error propagation with `?` and typed errors.
  - **Readability & DRY**: Doc comments on every `pub` item, early returns over nested blocks, unit tests colocated in `#[cfg(test)] mod tests`.
