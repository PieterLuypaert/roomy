# ruim

See where your disk space goes, right in the terminal.

```sh
cargo install --git https://github.com/PieterLuypaert/ruim
```

```sh
ruim                 # where the whole disk's space goes, by group
ruim ~/Documents     # browse any folder
ruim -p -d 2         # print the groups and their biggest places
ruim -p ~ | head     # print a folder's biggest entries
ruim --json -d 2 ~   # the same as JSON, two levels deep
```

Plain `ruim` scans the whole disk and sorts every byte into one group, such as apps,
caches, project dependencies or Docker, next to what macOS itself and free space take.
Open a group to see its biggest places, and a place to browse its folders. On a place
or an entry, `o` shows it in the Finder, `y` copies its path and `d` moves it to the
Trash after asking.

ruim counts the space files take on disk, counts hard links once and stays on one disk.
Folders macOS keeps private show up as "no access"; give your terminal Full Disk Access
(System Settings, Privacy & Security) to see inside them.
