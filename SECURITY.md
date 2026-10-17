# Security policy

## What counts as a security bug

The parsers in this project read untrusted files. Maps and mods come from other players, and
anyone can make a file that looks like a map. So a crafted or corrupted file must not be able to
harm the program that reads it.

We treat these as security bugs when a file causes them:

- a crash or a panic in any crate;
- memory or CPU use that grows far beyond the size of the file, or never ends;
- memory unsafety, such as reads or writes out of bounds, including in the Java and Python
  bindings;
- code execution of any kind. The Lua files are read as data and must never run as code.

An error such as `ParseError` for a bad file is the expected behavior, not a bug.

pzmap2img memory-maps input files. If another process shrinks a mapped file while it is being
read, the program crashes. This is a known limit, not a vulnerability. `--untrusted-input` reads
lotheaders and texture packs into memory instead, but lotpacks are always memory-mapped, so don't
let other processes modify a map while pzmap2img reads it.

## Reporting a vulnerability

Please report security bugs privately through GitHub's private vulnerability reporting:
<https://github.com/LtdJorge/pzmap-rs/security/advisories/new>. Do not open a public issue.

Include as much of this as you can:

- which crate and version you used, and from which language;
- the file that causes the problem, or a script that creates it;
- what happens, such as the panic message, the crash output, or how much memory it used;
- the command or code you ran.

Do not attach files from the game itself. If the problem needs a modified game file, send a
script or a description of the changes instead.

## Supported versions

The project is before version 1.0. Only the latest release gets security fixes.
