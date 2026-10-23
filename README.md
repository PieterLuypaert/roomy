# ruim

See where your disk space goes, right in the terminal.

```sh
cargo install --git https://github.com/PieterLuypaert/ruim
```

```sh
ruim                 # browse your home folder
ruim ~/Documents     # browse any folder
ruim -p ~ | head     # print the biggest entries
ruim --json -d 2 ~   # the same as JSON, two levels deep
```

ruim counts the space files take on disk, counts hard links once and stays on one disk.
Folders macOS keeps private show up as "no access"; give your terminal Full Disk Access
(System Settings, Privacy & Security) to see inside them.
