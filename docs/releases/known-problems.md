# Known problems in Opalro 0.2

What is known not to work, or not to work well yet, and what to do about each. The user guide's section 13, What v0.2 doesn't do yet, has the full list of what is still to come.

## Not there yet: a server, locking, repair, export to Git

Opalro 0.2 is for one person on one computer. It can't share a project, lock files for a team, or repair a damaged project: `opalro verify` finds damage but can't fix it. It can bring in a Git repository, but can't yet turn a project back into one; the import never changes the Git repository, so you can carry on in Git.

**What to do:** keep another copy of anything that matters, such as a copy of the whole project folder, `.opalro` included. If `opalro verify` reports damage, go back to that copy, and please tell us.

## Opalro 0.1 can't open a project that keeps conflicts

A project made with Opalro 0.1 opens in 0.2 as it is. To keep conflicts it needs `opalro setup --upgrade` once, which asks first; after that, Opalro 0.1 can't open it, and `opalro undo` can't turn it off. Opalro 0.1 can't open a project started with 0.2 either.

**What to do:** if you still need Opalro 0.1 for a project, copy the project folder, `.opalro` included, before the upgrade.

## Going back to another save is slower than Git on Linux

`opalro goto` takes about twice Git's time on Linux, often more: 1.93 to 2.74 times in eight runs on GitHub's Linux machines, how much depending on the machine's processor. Opalro's own bar is twice Git's time; this version is held to three and a half times instead, as 0.1 was.

**What to do:** nothing; it is only slower.

## `opalro who` is slower than `git blame`, and follows no rename in history from Git

On five files of Git's own history, `opalro who` took 3 to 5 times `git blame`'s time. In a project imported from Git it doesn't follow a file that was renamed, since the import records no moves: the lines from before the rename are named for the save that renamed the file.

**What to do:** for a file renamed in Git, `git blame` still works in the Git repository you imported from: the import never changes it.

## Files another program has open, on Windows

On Windows, a program that holds a file open (an editor, a game engine, a preview) stops Opalro from writing over it. The commands that write files in your folder (`opalro goto`, `opalro discard`, `opalro shelf`, `opalro unshelf`, `opalro undo`, `opalro redo` and `opalro setup --preset`) write everything else, name the file, and stop.

`opalro save` stops too, when the program doesn't let others read the file (Opalro says "Can't read" and the file's name), or writes to it while Opalro reads it (Opalro says it "changed while Opalro was reading it").

**What to do:** close that program. After an `opalro save`, save again; after any other command, run any command that changes the project, and it finishes what was left.

## Windows and macOS warn about Opalro

The programs aren't signed yet, so either system may warn the first time Opalro runs. The warning means only that nobody it knows vouched for the program.

**What to do:** check the download against `SHA256SUMS`, then follow the user guide's section 2, Installing, part "Windows and macOS warn about Opalro".

## Online-only files of sync tools other than OneDrive

Opalro works in a OneDrive folder. Other sync tools may show Opalro a placeholder, a file kept online with nothing on your disk yet. Opalro can't read one, so `opalro save` and `opalro status` stop at it.

**What to do:** make the project's files available offline in the sync tool (often "Always keep on this device" or "Make available offline"), then run the command again.

## USB sticks, SD cards and other FAT or exFAT drives

On Windows, a project on these drives can't be used in this version. They don't record who owns a folder, and Opalro can't tell their folders apart, so `safe-folders` can't vouch for one either. Opalro says "Opalro can't tell who owns the project at" the folder, "this drive doesn't record who owns folders", and that safe-folders can't vouch for a project on it. On Linux, Opalro was tried on both and works there; on macOS, it hasn't been tried yet. On every system, Opalro's last check before writing over a file compares fewer things on these drives than it can elsewhere.

**What to do:** on Windows, work on a copy of the project on your computer's own drive. Elsewhere, keep projects on your computer's own drive where you can.

## Some network shares

Some network shares (some NAS drives, Windows shares and WebDAV folders) give every folder the same identity. Opalro can't be sure which folder a `safe-folders` entry names there, so the entry doesn't count, and a project Opalro refuses there stays refused.

**What to do:** work on a copy of the project on a drive of your own.

## A project folder deep in other folders, on Windows

On Windows, a project can't be started or imported in a folder whose full path is longer than about 250 characters: Opalro stops with "The system cannot find the path specified" and leaves an unfinished `.opalro` folder there. No file of yours is changed. Files deeper inside a project are fine at any length.

**What to do:** delete that `.opalro` folder, move the project nearer the top of the drive, such as into `C:\Projects\`, and start it there.
