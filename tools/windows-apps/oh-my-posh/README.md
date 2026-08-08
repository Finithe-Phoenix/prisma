# Oh My Posh Windows x86-64 fixture

This fixture pins the official `posh-windows-amd64.exe` asset from Oh My Posh
`v30.6.3`. The executable is acquired from the immutable, versioned upstream
release URL recorded in `fixture.lock.json`; no script or test resolves a
`latest` URL.

The upstream executable is not committed to Prisma. Acquire and validate the
exact compatibility input from the repository root with:

```powershell
& .\tools\windows-apps\oh-my-posh\fetch.ps1
py -3.12 .\tools\windows-apps\oh-my-posh\validate_fixture.py --require-artifact
py -3.12 -m unittest discover .\tools\windows-apps\oh-my-posh -p "test_fixture.py" -v
```

The fetch script writes to the ignored `artifacts/` directory. It downloads to
a unique temporary file, validates both the locked byte size and SHA-256, moves
the verified file into place, and removes the temporary file on every exit
path. An existing mismatched artifact is never overwritten without `-Force`.

This fixture only establishes reproducible acquisition and provenance.
Executing `oh-my-posh.exe version` through Prisma and rendering a deterministic
prompt are separate compatibility gates.

## License and attribution

Oh My Posh is Copyright 2022 Jan De Dobbeleer and distributed under the MIT
License. `COPYING` is reproduced from the exact `v30.6.3` source tag. Prisma
does not modify or redistribute the downloaded executable as part of this
repository.
