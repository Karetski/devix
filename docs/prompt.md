# devix — Original brief

The original brief, kept verbatim as a frozen reference. The project has evolved since; current shape lives in `principles.md` and `roadmap.md`. This file is read-only history.

---

> okay, here's the idea i have. i want to build a basic, but high-performance, tui coding ide, with tree sitter and lsp support, and extensive plugin system (probably using lua or javascript). i want to use rust as a main programming language for the ide itself. i did my own research and found ox as an interesting reference. i don't want anything fancy just basic features like.
> 1. main code editor functionality with natural editing. code editor shows currently opened document. code editor has features like:
>  a. universal text input and navigation, non-modal.
>  b. using mouse inputs to navigate, select and scroll text.
>  c. scroll bars and line counts.
>  d. right click support for context menus.
>  e. basic clipboard management using ctrl+c, ctrl+v, ctrl+x, ctrl+z, etc. or using context menus.
>  f. code navigation and selection using:
>   i. ctrl+harrows to jump to the end/beginning of the line
>   ii. ctrl+varrows to jump top/bottom of the document.
>   iii. alt+harrows to jump words.
>   iv. same actions with shift added also select text.
>   v. multicursor editing and navigation using the same actions above. multicursor activates using ctrl+alt+varrows.
>  g. tabs support. new tabs are only opened on user's request using shift+ctrl+t. users can switch tabs using shift+ctrl+[ or shift+ctrl+]. when new document is opened, replace current tab content, not open a new tab.
>  h. code highlighting support. treesitter.
>  j. splits like tmux/ghostty.
>  k. go to next/previous opened document using ctrl+alt+harrows.
> 2. left and right panels hosting plugin contents. panels should be toggleable using shift+ctrl+alt+harrows. focus between left panel <> editor <> right panel
> 3. command palette which can be opened using ctrl+shift+p.
> 4. lsp support.
>  a. code completion.
>  b. go to definition using context menu or command palette.
>  c. build/run project.
>  d. symbols(outline) navigation using ctrl+shift+o (same display style as command palette). ctrl+o for local document symbols.
>  e. documentation using context menu or command palette..
> 5. theme support.
> 6. settings page with things like shortcut config, plugin settings, themes, etc.
> 7. plugin support. plugins can:
>  a. contribute actions and shortcuts
>  b. render tui in left or right panels.
>  c. open documents in the editor (main split).
>  d. contribute custom editors.
