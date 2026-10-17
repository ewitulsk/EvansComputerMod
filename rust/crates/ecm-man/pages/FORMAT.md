# Manual page format

Every `*.txt` file in this folder is one page of `man` (and the source of
`help <topic>`). The file name is the page name: what the player types after
`man` (`ls.txt` is `man ls`). Command pages are named after the command the
player types; feature pages (topics) after the feature (`shell`, `keys`,
`networking`, ...). Pages are compiled into `man` and `help`
(`ecm-man/build.rs`). A computer can add its own pages as
`/usr/share/man/<name>.txt` in the same format; they win over built-in ones.

Plain ASCII only. Lines at most 76 characters.

## Layout

```
NAME
    ls - list the files in a directory

SYNOPSIS
    ls [-l] [path]

DESCRIPTION
    Lists the files and directories in path (the current directory if
    none is given), sorted by name. Directories end with a slash.

OPTIONS
    -l
        Long listing: a d or f column and the size of each file.

EXAMPLES
      ls
      ls -l /home

SEE ALSO
    cd, pwd, cat, files
```

- A **heading** is a line that starts in column 0 and is all capitals
  (letters, digits, spaces). Headings are drawn in bold.
- **NAME** is required and its first line is `name - one-line summary`
  (a plain hyphen with a space on each side). `help` uses the summary.
- **SYNOPSIS** is the command line(s); one per line, indented 4.
- Text indented **exactly 4 or exactly 8** spaces is a paragraph: the pager
  joins its lines and re-wraps them to the terminal's width (a phone is 30
  columns, a Terminal 80 or 160), keeping the indent. A blank line ends a
  paragraph. Use indent 8 for the description under an option or key.
- A line with **any other indent** (2, 6, 10, ...) is shown exactly as
  written: examples, tables, diagrams. Keep these short (they are not
  re-wrapped on narrow screens). Examples go at indent 6.
- Inline emphasis: none; write commands and file names as they are.
- Usual headings, in this order when present: NAME, SYNOPSIS,
  DESCRIPTION, OPTIONS (or COMMANDS / KEYS), FILES, EXAMPLES, NOTES,
  SEE ALSO. Topic pages can use their own headings.
- SEE ALSO lists other page names separated by commas.

Write what the program really does (read its source and usage strings),
in plain words for a Minecraft player who may never have used a Unix
shell, but with the precision of a real man page.
