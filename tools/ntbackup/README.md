# ntbackup — read Notes Tech backups on a desktop

`ntbackup` decrypts a Notes Tech encrypted backup (`.ntbackup`, created from **Settings → Create a
backup** in the app) and writes your notes as Markdown: either a plain folder tree, or pages synced into
a [Logseq](https://logseq.com) graph.

Everything happens locally. The tool never touches the network.

> ⚠️ **The output is plaintext.** The `.ntbackup` file is encrypted; the Markdown it produces is not.
> Notes from vaults are decrypted too, when you give their passphrase.

## Install

```sh
cargo install --path tools/ntbackup
```

## Getting backups onto your computer

In the app, **Settings → Backup folder** lets you choose a folder once, for example a folder of the
Nextcloud app. Every backup is then written there as a new `notes-tech-YYYYMMDD-HHMMSS.ntbackup` file.
The provider app (Nextcloud, or a sync app for a local folder) uploads it, and your desktop sync client
brings it down. The Notes Tech app itself has no network permission.

## Try it with the app you already have

The backup and its "choose where to save" picker are in the app since **3.2.0**. The remembered backup
folder above only saves you from picking the location each time. To test the tool today:

1. **On the phone:** **Settings → Create a backup**. Use **Create a backup**, not **Export**: Export
   writes an unencrypted ZIP of Markdown files, which this tool does not read. Write down the code the
   app shows, or choose a passphrase of your own. When the picker opens, go to the Nextcloud app's
   storage (or any folder that syncs to your computer) and save there.
2. **On the computer:** once the file has synced, build the tool and run it:

   ```sh
   cd path/to/notes_tech
   cargo install --path tools/ntbackup
   # or, without installing: cargo run --manifest-path tools/ntbackup/Cargo.toml -- <command> …

   ntbackup inspect ~/Nextcloud/…/notes-tech-2026-11-08.ntbackup
   ntbackup export  ~/Nextcloud/…/notes-tech-2026-11-08.ntbackup -o ~/notes-md
   ntbackup export  ~/Nextcloud/…/notes-tech-2026-11-08.ntbackup --logseq -o ~/path/to/graph --dry-run
   ```

   - It asks for the backup's code or passphrase at a hidden prompt; case, spaces and dashes don't
     matter.
   - It then asks for the passphrase of each passphrase vault. Leave it empty to skip that vault.
   - For Logseq, `-o` is the graph's root: the folder that holds `pages/` and `logseq/config.edn`.
     Run with `--dry-run` first: it lists what it would create or change and writes nothing.

If something fails, the [exit code](#exit-codes) says why:
- **4:** wrong secret, or a damaged file.
- **3:** not a backup this tool knows.
- **1:** anything else.

## Usage

```sh
# What the header says (no secret needed)
ntbackup inspect notes-tech-20261108-101500.ntbackup

# Markdown, one file per note, folders as nested directories
ntbackup export notes-tech-20261108-101500.ntbackup -o ~/notes-md

# Pages in your Logseq graph (see below)
ntbackup export notes-tech-20261108-101500.ntbackup --logseq -o ~/logseq --dry-run
ntbackup export notes-tech-20261108-101500.ntbackup --logseq -o ~/logseq

# The decrypted JSON content
ntbackup decrypt notes-tech-20261108-101500.ntbackup -o content.json
```

**The backup secret** is asked for at a hidden prompt. That's the 24-character code the app showed you
(`K7QM-T2XB-…`; case, spaces and dashes don't matter, and `O`/`I`/`L` are read as `0`/`1`), or the
phrase you chose. For scripts, `--secret-stdin` reads it from the first line of standard input. There
is deliberately no option to pass it as an argument, because arguments end up in shell history.

**Vaults:**
- **Passphrase vaults:** you are asked for each vault's passphrase. Leave it empty to skip that vault.
  `--skip-vaults` never asks.
- **PIN vaults:** if the vault was open when the backup was made, it is in the backup with its key, and
  its notes are exported without asking.

**What is left out:** notes in the trash, and notes of vaults you skipped. The summary counts both.

### Plain export (`-o DIR`)

The same format as the app's own **Export all**:
- one `.md` file per note, with YAML frontmatter (`title`, `folder`, `tags`, `created`, `updated`,
  `pinned`, `favorite`), readable by Obsidian, Bear, Foam and others;
- notes from a vault are marked ` [unlocked]`;
- the one difference: folders are **nested** (`Work/Projects/Plan.md`), where the app flattens them.

`DIR` must be empty; `--force` writes into it anyway. On Linux and macOS, files are created readable by
you only.

Zipping `DIR` and importing the zip in the app (**Settings → Import**) brings the notes back. The
importer only reads the first directory level, so nested folders come back one level deep.

### Logseq (`--logseq -o GRAPH`)

`GRAPH` is the root of an existing Logseq graph (the folder holding `pages/` and `logseq/config.edn`).
Each note becomes a page in `pages/`:

```
title:: Plan: Q3?
folder:: [[Work/Projects]]
tags:: work, project-x
created:: 2026-11-08T10:00:00.000
updated:: 2026-11-08T10:30:00.000
pinned:: true
ntbackup-id:: 3f2a…
ntbackup-hash:: 9c1e…

- TODO draft the plan
- DONE call Ana
```

- **Page name = note title**, so the `[[links]]` between your notes, and their backlinks, work in
  Logseq. The folder is a `folder::` property; each folder gets its own page, and nested folders appear
  as a namespace.
- **Checklists become Logseq tasks:** `- [ ]` becomes `TODO`, `- [x]` becomes `DONE`. Code blocks are
  left alone.
- **Two notes with the same title:** the later one is suffixed ` (2)`.
- **A page of yours with the same name:** the note is written as `Title (notes-tech)`.

**Running it again syncs one way, from the backup to the graph:**
- Only pages carrying `ntbackup-id::` are ever rewritten, renamed or deleted. Your other pages are never
  touched.
- A page whose note was renamed is renamed. A page whose note was deleted, or moved to the trash, is
  deleted.
- Pages of vaults you skipped this time are kept.
- **A page you edited in Logseq is left as it is**, with a warning, because its `ntbackup-hash::` no
  longer matches. `--overwrite-edited` replaces it anyway. Edits made in Logseq never flow back to the
  app.
- `--dry-run` lists what would change and changes nothing.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Done |
| 1 | Any other error (I/O, unreadable content, output refused) |
| 2 | Usage error |
| 3 | Not a backup, or a format/parameters this tool does not know. Checked before any key is derived. |
| 4 | Wrong secret, or damaged file. AES-GCM cannot tell them apart. |

## Format

The format is described in the app's source:
- `app/src/main/java/com/filestech/notes_tech/security/sauvegarde/FichierDeSauvegarde.kt` (header and
  encryption);
- `app/src/main/java/com/filestech/notes_tech/data/sauvegarde/ContenuDeSauvegarde.kt` (JSON content).

In short:
- a 48-byte header (`NTBACKUP`, version 1, Argon2id t=3 m=64 MiB p=1, salt, nonce);
- then AES-256-GCM of the JSON, with the header as associated data.

The tests replay the app's parity vectors (`app/src/test/resources/parite/`), which were produced by
running the app's original code, so the vault crypto is checked byte for byte.

## Development

```sh
cd tools/ntbackup
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```
