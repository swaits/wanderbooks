# File names the viewer writes

Governs `ledger_gui::export::file_name`. Fetched 2026-10-14.

## Microsoft, "Naming Files, Paths, and Namespaces" (learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file)

> Use any character in the current code page for a name, including
> Unicode characters and characters in the extended character set
> (128–255), except for the following: The following reserved characters:
> < (less than) > (greater than) : (colon) " (double quote) / (forward
> slash) \ (backslash) | (vertical bar or pipe) ? (question mark) *
> (asterisk) Integer value zero, sometimes referred to as the ASCII NUL
> character. Characters whose integer representations are in the range
> from 1 through 31, except for alternate data streams where these
> characters are allowed.

> Do not use the following reserved names for the name of a file: CON,
> PRN, AUX, NUL, COM1, COM2, COM3, COM4, COM5, COM6, COM7, COM8, COM9,
> COM¹, COM², COM³, LPT1, LPT2, LPT3, LPT4, LPT5, LPT6, LPT7, LPT8, LPT9,
> LPT¹, LPT², and LPT³. Also avoid these names followed immediately by an
> extension; for example, NUL.txt and NUL.tar.gz are both equivalent to
> NUL.

> Do not end a file or directory name with a space or a period. Although
> the underlying file system may support such names, the Windows shell and
> user interface does not. However, it is acceptable to specify a period
> as the first character of a name.

## POSIX (IEEE Std 1003.1-2017, `<limits.h>`)

`{NAME_MAX}`: "Maximum number of bytes in a filename (not including the
terminating null of a filename string)"; Linux's common file systems set
it to 255, and NTFS limits a name to 255 UTF-16 units. wanderbooks keeps a
name within 255 bytes of UTF-8, which fits both.
