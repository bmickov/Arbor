# Arbor

A Git client for Linux, built around the workflows that make Tower pleasant:
drag-and-drop merging and rebasing, commit reordering, line-by-line staging,
and an undo button that covers almost everything.

Arbor is an original app. It isn't affiliated with Tower or fournova and
contains none of their code or artwork.

## Features

**Working copy**
- Stage, unstage or discard whole files, single hunks, or individual lines
  (click a line, shift-click or drag to extend, ctrl-click to toggle)
- Drag files between the Unstaged and Staged lists
- Commit composer with amend, skip-hooks, and a per-repo saved draft
- Runs your `pre-commit`, `commit-msg` and `post-commit` hooks (respects `core.hooksPath`)
- Merge conflicts: keep mine, keep theirs, or mark resolved after editing; abort

**History**
- Commit graph for all branches, the current branch, or any single branch
- Commit details with per-file diffs, unified or side by side, and image previews
- File history (follows renames) and blame
- Right-click a commit to check out, branch, tag, cherry-pick, revert, reset,
  edit its message, squash or fixup into its parent, or remove it
- **Drag a commit up or down to reorder it**
- "Rewrite from here…" opens an interactive rebase editor: reorder by drag,
  then keep, edit message, squash, fixup or remove each commit

**Branches and remotes**
- **Drag a branch onto another branch** to merge or rebase
- **Drag a branch onto a remote** to push it there
- Drag a commit onto the current branch to cherry-pick it
- Fetch (with prune), pull (merge or rebase), push with rejection handling and force push
- SSH via ssh-agent or `~/.ssh/id_*` keys; HTTPS via your git credential helper,
  or a username and token prompt
- Clone, init, add and remove remotes, track upstream branches, push and delete tags

**Stashes**
- Stash with a message, optionally including new files or keeping staged changes
- Apply, apply-and-remove, delete, or turn a stash into a branch
- Drag a stash onto Changes to apply it

**Undo**
- Ctrl+Z undoes commits, amends, staging, discards (even of untracked files),
  merges, rebases, history rewrites, cherry-picks, reverts, resets, checkouts,
  branch and tag creation, deletion and renames, stash operations, and remote changes
- The undo history lives in `.git/arbor/undo.json`, so it survives restarts
- Undo refuses rather than guesses when the repository changed since the operation

**Safety**
- Merges, rebases and rewrites refuse to start over uncommitted changes, and
  offer to stash them, run the operation, and put them back
- Rebases are replayed in memory: if any commit conflicts, nothing changes
- Rewriting commits that are already pushed asks for confirmation first

## Install

Pick the package for your distribution from the release files:

```sh
# Debian, Ubuntu, Mint, Pop!_OS
sudo apt install ./Arbor_0.1.0_amd64.deb

# Fedora, openSUSE
sudo dnf install ./Arbor-0.1.0-1.x86_64.rpm

# Anything else
chmod +x Arbor_0.1.0_amd64.AppImage && ./Arbor_0.1.0_amd64.AppImage
```

Run `arbor /path/to/repo` to open a repository directly. Without an argument,
Arbor reopens the last repository you used.

## Keyboard

| Keys | Action |
| --- | --- |
| Ctrl+Z | Undo the last operation |
| Ctrl+Enter | Commit (in the message fields) |
| Ctrl+R | Refresh |
| Ctrl+1 / 2 / 3 | Changes / History / Stashes |
| ↑ ↓ | Move through files or commits |
| Space or Enter | Stage or unstage the focused file |

## Build from source

Requirements: Rust 1.80+, Node 20+, and the Tauri system libraries.

```sh
# Debian/Ubuntu
sudo apt install libwebkit2gtk-4.1-dev build-essential libssl-dev \
  libayatana-appindicator3-dev librsvg2-dev

npm install
npm run tauri dev      # run with hot reload
npm run tauri build    # .deb, .rpm and AppImage in src-tauri/target/release/bundle
```

Backend tests run against throwaway repositories:

```sh
cd src-tauri && cargo test
```

## How it's built

- `src-tauri/src/git/` — all Git logic, on libgit2 via the `git2` crate
  (`vendored-libgit2`, so it doesn't depend on the system's libgit2 version).
  Each module is stateless: it opens the repository, does one job, returns data.
- `src-tauri/src/lib.rs` — Tauri commands; libgit2 work runs on worker threads
- `src/` — Svelte 5 UI. `lib/state.svelte.ts` holds app state and the `run()`
  wrapper (busy state, errors, auth prompts, auto-stash, refresh);
  `lib/actions.ts` holds user-facing operations; `lib/drag.svelte.ts` is a
  pointer-based drag-and-drop layer (HTML5 drag-and-drop is unreliable in WebKitGTK).

## Known limits

- Commit signing (GPG/SSH) isn't supported yet; commits are unsigned.
- Rebases that hit a conflict stop without changing anything instead of
  pausing for you to resolve. Merge instead, or finish that rebase with the git CLI.
- Rebasing across merge commits isn't supported.
- Submodules, worktrees, LFS and bisect aren't handled in the UI.
- Pull requests and hosting-service integrations aren't included.
