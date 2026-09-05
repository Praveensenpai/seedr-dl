# Rust Codebase Rules

Drop this in as `CLAUDE.md` / `.cursor/rules` / `RULES.md`. Goal: every file readable in one sitting, every module has one job, nothing hidden behind `#[allow(...)]`.

---

## 1. Hard limits (enforced, not suggested)

| Thing | Limit | Why |
|---|---|---|
| Lines per file | 300 (soft), 400 (hard fail) | if it's bigger, it's doing 2+ jobs — split it |
| Lines per function | 40 (soft), 60 (hard fail) | anything longer needs sub-functions |
| Functions per file | 8–10 | more means the file owns too many responsibilities |
| Params per function | 4 (use a struct beyond that) | param soup = missing abstraction |
| Nesting depth | 3 | use early returns / `?` instead of pyramid code |
| Files per folder (module) | ~7 (soft) | beyond that, introduce a subfolder |
| Cyclomatic complexity | 10 (clippy `cognitive_complexity`) | |

Put these in CI, not just in your head — see §6.

---

## 2. Zero tolerance list

These are **never** allowed, no exceptions, no "just this once":

- `#[allow(dead_code)]`, `#[allow(unused)]`, `#[allow(warnings)]` — if code is dead, **delete it**. Git history is your undo button, not your codebase.
- `#[allow(clippy::...)]` at file/crate level to silence a category — fix the lint or, in the rare justified case, `#[allow(clippy::specific_lint)]` on the *single line*, with a `// reason:` comment above it.
- `unwrap()` / `expect()` outside of tests and `main()`'s top-level error boundary. Use `?` and a real error type.
- `.clone()` used to dodge a borrow-checker error you don't understand. Either the ownership model is wrong, or you actually need the clone — know which.
- Commented-out code blocks. Delete or `git stash`/branch it.
- `mod.rs` files (old-style) — use `foldername.rs` next to `foldername/` (Rust 2018+ style).
- `TODO`/`FIXME` without a linked issue number.
- Catch-all `Error` enums with a single `Other(String)` variant used everywhere — defeats the purpose of typed errors.
- `pub` on anything that doesn't need to cross the module boundary. Default to private, widen only when forced.

Build must fail on `cargo build` with **zero warnings**. Treat every `warning:` as a `error:` — see `deny.toml` / lint config in §6.

---

## 3. Folder structure — organize by *role*, not by type

Don't do this (organizing by technical layer — becomes a junk drawer per folder):
```
src/
  models/
  utils/
  helpers/
  handlers/
```

Do this (organizing by domain/feature — each folder is a bounded responsibility):
```
src/
  main.rs                 # wiring only: parse args, init logging, call app::run()
  lib.rs                  # crate root, re-exports public API
  app.rs                  # top-level orchestration / run loop

  domain/                 # pure business logic, no I/O, no framework types
    user.rs
    order.rs
    pricing.rs

  infra/                  # side-effecting adapters: DB, HTTP clients, filesystem
    db/
      mod.rs              # trait defs (ports)
      postgres.rs         # impl
    http_client.rs

  api/                    # entrypoints: HTTP handlers, CLI commands, gRPC services
    routes/
      users.rs
      orders.rs
    middleware.rs

  config.rs                # env/config loading only
  error.rs                 # crate-wide error type(s)
```

Rules for this layout:
- **`domain/`** never imports from `infra/` or `api/`. It depends on nothing but the standard library + small pure crates (serde, etc). This is what makes it testable without mocks.
- **`infra/`** implements traits defined in `domain/` (ports-and-adapters / hexagonal). Swapping Postgres for SQLite = new file in `infra/db/`, zero changes elsewhere.
- **`api/`** is thin: parse request → call domain/app function → serialize response. No business logic lives here.
- A folder gets a `foo/foo.rs`-style split the moment `foo.rs` crosses ~200 lines **or** starts having 2+ unrelated concerns — not before (don't pre-split empty scaffolding).
- One `struct`/`enum` + its `impl` blocks per file when the type is non-trivial. Small tightly-coupled types (e.g. a 3-line newtype + its `From` impl) can share a file.

---

## 4. DRY, without overdoing it

- **Rule of three**: don't abstract on the 2nd occurrence, do it on the 3rd. Premature abstraction is its own form of duplication (duplicated *complexity*).
- Shared logic goes in the lowest common module both callers can see — not in a `utils.rs` grab-bag. If you're about to create `utils.rs`, ask "utility for *what*?" and name it that (`string_fmt.rs`, `retry.rs`, `time.rs`).
- Use traits to unify behavior across types instead of copy-pasting near-identical functions per type.
- Generics/macros are a last resort for DRY, not a first one — a macro that saves 5 lines but costs 5 minutes of reading is a net loss. Prefer a plain function or trait first.
- Config/constants defined once (`const`/`static` in a single `constants.rs` or colocated with the domain that owns them) — never duplicate a magic number across files.

---

## 5. Readability rules

- Function names are verbs (`calculate_total`, `fetch_user`), types are nouns (`Order`, `UserId`).
- Every `pub fn` and `pub struct` gets a `///` doc comment — one line minimum, explaining *why* it exists, not just restating the name.
- Early return over nested `if`:
  ```rust
  // bad
  fn f(x: Option<i32>) -> i32 {
      if let Some(v) = x { if v > 0 { return v; } else { return 0; } } else { return -1; }
  }
  // good
  fn f(x: Option<i32>) -> i32 {
      let Some(v) = x else { return -1; };
      if v <= 0 { return 0; }
      v
  }
  ```
- Prefer `match`/`?`-based control flow over `.unwrap_or_else()` chains more than 2 deep.
- No more than one level of `.iter().map(...).filter(...).collect()` chaining without an intermediate named variable if it hurts readability — name the intermediate steps.
- Tests live in the same file under `#[cfg(test)] mod tests { ... }` for unit tests; integration tests go in `tests/`. A file without any test coverage for non-trivial logic is a smell, not a rule violation per se — but domain/ logic should be tested.

---

## 6. Enforce it in CI (don't rely on discipline alone)

`Cargo.toml`:
```toml
[lints.rust]
unused = "deny"
dead_code = "deny"
warnings = "deny"

[lints.clippy]
all = "deny"
pedantic = "warn"
cognitive_complexity = "warn"
too_many_arguments = "deny"
unwrap_used = "deny"
expect_used = "deny"
```

CI step:
```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build 2>&1 | grep -q "warning:" && exit 1
cargo test
# optional: enforce file/function line limits with tokei or a small script
```

A tiny line-count gate (run in CI, fail on violation):
```bash
find src -name '*.rs' | xargs wc -l | awk '$1 > 400 {print; found=1} END {exit found}'
```

---

## 7. Quick checklist before a PR/commit

- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] No `#[allow(...)]` added anywhere
- [ ] No file > 400 lines, no function > 60 lines
- [ ] New logic lives in `domain/` if it's business rules, `infra/` if it touches the outside world, `api/` if it's an entrypoint
- [ ] No copy-pasted block that exists elsewhere — check before writing, not after
- [ ] Every new `pub` item has a doc comment
- [ ] No dead code left "just in case"
