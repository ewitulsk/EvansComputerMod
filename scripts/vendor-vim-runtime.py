"""Copy the part of Vim's runtime the computers ship into rust/third_party/vim/runtime
(see rust/third_party/vim/README.ecm.md):

    python3 scripts/vendor-vim-runtime.py <vim release>/runtime rust/third_party/vim/runtime

Then regenerate runtime/doc/tags with Vim's :helptags."""
import os, shutil, sys, glob

SRC = sys.argv[1]  # vim-9.2.1202/runtime
DST = sys.argv[2]  # rust/third_party/vim/runtime

LANGS = ("c cpp python python2 lua rust sh java javaid javascript json jsonc markdown html css xml "
         "yaml toml vim help conf diff make sql sqloracle dosini gitcommit git go ruby perl pod "
         "text man ctrlh csv dockerfile tcl scheme vb lisp haskell kotlin zig gitconfig "
         "gitrebase readline tmux zsh awk sed asm qf bash tex plaintex initex").split()

files = [
    "defaults.vim", "filetype.vim", "ftoff.vim", "ftplugin.vim", "ftplugof.vim",
    "indent.vim", "indoff.vim", "scripts.vim", "optwin.vim", "vimrc_example.vim",
    "evim.vim", "mswin.vim",
    "plugin/matchparen.vim", "plugin/tohtml.vim", "plugin/manpager.vim",
    "autoload/tohtml.vim",
    "syntax/syntax.vim", "syntax/synload.vim", "syntax/syncolor.vim", "syntax/nosyntax.vim",
    "syntax/manual.vim", "syntax/2html.vim",
    "spell/en.utf-8.spl",
]
for d in ["autoload/dist", "colors", "pack/dist/opt/matchit", "pack/dist/opt/comment",
          "pack/dist/opt/nohlsearch", "pack/dist/opt/justify"]:
    for root, _, names in os.walk(os.path.join(SRC, d)):
        for n in names:
            files.append(os.path.relpath(os.path.join(root, n), SRC).replace(os.sep, "/"))
for lang in LANGS:
    for kind in ("syntax", "indent", "ftplugin", "autoload"):
        p = f"{kind}/{lang}.vim"
        if os.path.exists(os.path.join(SRC, p)):
            files.append(p)
    for p in glob.glob(os.path.join(SRC, "autoload", lang, "*.vim")):
        files.append(os.path.relpath(p, SRC).replace(os.sep, "/"))

DOCS = ("help intro starting editing motion scroll insert change undo repeat visual various "
        "recover cmdline options pattern map tagsrch quickref index windows tabpage syntax "
        "spell diff autocmd filetype indent fold tips message usr_toc usr_01 usr_02 "
        "usr_03 usr_04 usr_05 usr_06 usr_07 usr_08 usr_09 usr_10 usr_11 usr_12 usr_20 usr_21 "
        "usr_22 usr_23 usr_24 usr_25 usr_26 usr_27 usr_28 usr_29 usr_30 usr_31 usr_32 usr_40 "
        "usr_41 uganda vi_diff builtin eval userfunc vim9 term mbyte digraph helphelp "
        "print pi_paren").split()
for d in DOCS:
    files.append(f"doc/{d}.txt")

seen = set()
n = size = 0
for f in files:
    if f in seen:
        continue
    seen.add(f)
    s = os.path.join(SRC, f)
    if not os.path.isfile(s):
        print("missing", f)
        continue
    t = os.path.join(DST, f)
    os.makedirs(os.path.dirname(t), exist_ok=True)
    shutil.copyfile(s, t)
    n += 1
    size += os.path.getsize(s)
print(n, "files", size // 1024, "KiB")
