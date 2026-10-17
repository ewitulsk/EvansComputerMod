# Browser

The in-game web browser: **Browser** (`dev.ecm.browser`), one app for the Terminal desktop and the ePhone, built on a
reusable web engine, `ecm-web`. Pages come over the in-game network (`http://`, HTTP/1.0) and from the computer's disk
(`file://`), so a player can write a site with `edit`, preview it locally, serve it with `httpd 80` and visit it from
another computer or a phone.

Decided with Evan (2026-10-17): an HTML subset as the page format, the full CSS cascade with flexbox in the first CSS
release, basic JavaScript through QuickJS later. Per-host session cookies; `httpd` gets a CGI mode and Python/Lua get a
server module; the first milestone is CSS layout on the village sites and `file://`, not waiting for DNS.

## Contents

- [Phases](#phases)
- [Using it](#using-it)
- [The engine: ecm-web](#the-engine-ecm-web)
- [HTML](#html)
- [CSS](#css)
- [The app](#the-app)
- [httpd](#httpd)
- [Scenarios](#scenarios)
- [Tests](#tests)
- [Limits and open items](#limits-and-open-items)

## Phases

One PR per phase.

| Phase | What | Scenario | State |
|---|---|---|---|
| 1 | `ecm-web` fetch + parse + CSS cascade + block/inline layout + flexbox; the app on desktop and phone; village sites; `file://` | `browser_village`, `browser_phone`, `browser_local_site` | **built** (this page) |
| 2 | Positioning, images, tables, forms, cookies in use; `httpd` CGI mode and virtual hosting; the Python/Lua server module; bookmarks, history and a home page setting | `browser_form` | planned |
| 3 | JavaScript: QuickJS (rquickjs, built with wasi-sdk like Lua), the DOM subset, events, timers, `fetch()`, `localStorage` | `browser_js` | planned |
| 4 | Names (after DNS, PR #60): `http://web.v4.ring/`, registrar pages | `browser_dns` | planned |

## Using it

On a computer, start the desktop with `gui` and open **Browser** from the launcher, or run `browser [address]` from the
desktop's Terminal. On an ePhone it is on the home grid. It is a system app on every computer (`SystemApps`, permissions
`network` and `files:documents`, allowed at install).

The address bar takes:

| Typed | Opens |
|---|---|
| `100.65.0.10`, `10.0.0.5:8080/page.html` | `http://` that address |
| `http://…` (`https://` is taken as `http://`: there is no TLS in the game) | as typed |
| `/home/site/index.html`, `file:///home/site/` | a file, or a folder (its `index.html`, or a listing) |
| `about:home` | the start page: the ten Tech Village sites and the player's files |

Desktop (medium and wide windows): `[<] [>] [Reload] [Home] [address] [Go]` over the page, and a status line that shows
the link under the pointer or what is loading. Phone (compact): the address on top, `[Back] [Fwd] [Reload] [Home]` in a
bar at the bottom, in thumb reach.

Input: click or tap a link; the wheel, a drag (a finger on the phone), arrows, Page Up/Down, Space, Home and End scroll;
Alt+Left / Alt+Right or Backspace go back and forward; F5 reloads; Ctrl+L goes to the address bar. In hand on an ePhone,
hotkey slot 1 is Back, 2 scrolls up a page, 3 down. A suspend saves the history and the scroll position; the page opens
again after a cold start.

Errors are pages: "Can't reach this page" (nobody answered), "Can't find the server" (a name that doesn't resolve), "No
network", "Not allowed" (the permission was taken away in Settings), "File not found". A server's own error (a 404) is
shown as the server sent it.

## The engine: ecm-web

`rust/crates/ecm-web`, sans-IO (no host calls) and tested on the host. A page goes through five stages:

1. **Fetch** (`fetch.rs`): `http://` as HTTP/1.0 with `Host:`, `Content-Length` and chunked bodies, redirects
   301/302/303/307/308 (at most 5), GET and form POST (`application/x-www-form-urlencoded`), per-host session cookies
   (`Max-Age=0` deletes); `file://` from the disk. The I/O is a `Transport` the program supplies: the computer's TCP
   sockets and files in `browser`, scripted ones in tests. `url.rs` parses and resolves addresses and does form encoding.
2. **Parse** (`html.rs`, `dom.rs`): a forgiving tokenizer and tree builder into a small DOM (an arena of nodes). Unknown
   tags keep their text; no input makes it fail. Elements nest at most 96 deep (deeper start tags become siblings), so no
   page can overflow the layout's stack.
3. **Style** (`css.rs`, `style.rs`): the cascade into a computed style per element.
4. **Layout** (`layout.rs`): block flow, inline text broken into lines with the ECM font metrics, flexbox, relative offsets.
5. **Paint** (`paint.rs`): a display list of rects and single-line texts in page coordinates, plus hit boxes that map a
   point back to its DOM node (links, `:hover`, later form controls).

`page.rs` strings them together: `Page::parse`, `load_sheets` (linked sheets fetched through a closure), `render` for a
viewport, `link_at`, `anchor`.

## HTML

| Kind | Elements |
|---|---|
| Structure | `html head title body div section header footer nav main article aside p br hr` |
| Text | `h1`–`h4` (32/24/16/16 lu), `a`, `span`, `code`, `pre`, `b strong em i u s small big blockquote mark` |
| Lists, tables | `ul ol li dl dt dd`; `table tr td th` (phase 1: a row is a flex row of equal cells; proper tables are phase 2) |
| Media | `img src alt width height` (phase 1 draws the alt text in a frame; pictures are phase 2) |
| Forms | drawn (`input` text/password/checkbox/radio/submit, `textarea`, `select`, `button`); working forms are phase 2 |
| Dropped | `script`, `style`, `head`, `iframe`, `video`, `canvas`, … are not drawn (their text isn't either) |

Implied ends as browsers do them: a block start closes an open `<p>`, a new `<li>` closes the previous one, likewise
`<option>`, `<tr>`, `<td>`/`<th>`, `<dt>`/`<dd>`. Character references: `&amp;`, `&#65;`, `&#x41;` and the common names.
Legacy attributes: `bgcolor`, `<font color>`, `align`, `width`/`height` on images and cells, `hidden`.

## CSS

- **Sources**: `style=""`, `<style>` (with `media`), `<link rel=stylesheet>` (a sheet that doesn't load is skipped: the page
  shows unstyled), `@media` with `min-`/`max-width` and `-height`, `screen`, `all`, `print`, `not`, `only`, `and` and
  lists. `@supports` blocks apply; other at-rules are skipped.
- **Cascade**: the browser's own sheet, then the page's sheets in document order, then `style=""`; ordered by importance
  (UA normal < author normal < author `!important` < UA `!important`), specificity, source order; inheritance;
  `inherit`, `initial`, `unset`.
- **Selectors**: `tag`, `.class`, `#id`, `*`, `[x]`, `[x=y]` (and `~= |= ^= $= *=`), descendant, `>`, `+`, `~`,
  `:hover`, `:focus`, `:active`, `:first-child`, `:last-child`, `:only-child`, `:nth-child()`, `:nth-last-child()`,
  `:root`, `:link`, `:checked`, `:disabled`, `:enabled`, `:empty`, `:not()`. Anything else (pseudo-elements like
  `::before` included) makes that selector never match.
- **Box model**: `margin` (with `auto` centring), `padding`, `border` (width, colour, style none/solid; every style draws
  solid), `border-radius`, `width`/`height`, `min-`/`max-`, `box-sizing`, `background-color` (the `background`
  shorthand's colour), `opacity`, `overflow: hidden|auto` (clips).
- **Text**: `color`, `font-size` (px, em, rem, %, keywords; computed in px and drawn at the nearest ECM size: 8, 12, 16,
  24, 32), `font-family: monospace`, `font-weight` (bold is drawn twice, one lu apart), `font-style` (accepted, drawn
  upright: there is no italic font), `text-align`, `line-height`, `text-decoration` (underline, line-through),
  `text-transform`, `text-indent`, `white-space` (normal, nowrap, pre, pre-wrap, pre-line), `list-style-type`, `visibility`.
- **Layout**: `display: block|inline|inline-block|none|flex|inline-flex|list-item` (`table-row` is a flex row, `grid`
  and the other table values are blocks); flexbox: `flex-direction` (and reverse), `flex-wrap`, `justify-content`,
  `align-items`, `align-self`, `gap`/`row-gap`/`column-gap`, `flex-grow`/`-shrink`/`-basis` and the `flex` and
  `flex-flow` shorthands, `order`, auto margins on items, `min-width: auto` (an item isn't squeezed below its longest
  word); `position: relative` (absolute and fixed are laid out in flow until phase 2).
- **Units**: `px % em rem vw vh vmin vmax pt pc in cm mm ch ex`, `calc()` with `+` and `-`. **Colours**: names, `#rgb`,
  `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`/`rgba()` (comma or space syntax), `hsl()`/`hsla()`, `transparent`,
  `currentcolor`.
- **Theme**: unstyled pages use the shell theme's colours (text, background, accent for links, borders, control
  surfaces), so they look native in light and dark; a theme change re-renders the page.
- Not in v1: grid, floats (blocks), transforms, transitions and animations, custom fonts, `::before`/`::after`.

The mono font exists at 16 and 32 lu only: preformatted text (`pre`, `white-space: pre`) uses it at 16 or 32; small
inline `code` keeps the ui font at the paragraph's size. Characters the Latin-1 fonts don't have become their nearest
text (curly quotes, dashes, `…`, arrows, `•` as `·`).

## The app

`rust/wasm-programs/browser`: the `browser_app` library (tested on the host) and the `browser` program.

- `model.rs`: the page shown, history (50 entries; a new visit drops the forward history), scroll, the pointer (hover,
  press and drag), what to load next.
- `view.rs`: the window from the toolkit's widgets (`ecm-ui`), the size class picking the desktop or phone layout. The page
  is one custom area; its size is what the bars leave.
- `scene.rs`: the engine's display list as scene nodes: the page background, one group moved by the scroll offset holding
  only what is on screen (and 64 lu around it), clip and opacity groups, and a scrollbar. Scrolling changes one group's Y
  plus the nodes entering and leaving.
- `app.rs`: `BrowserApp`: handlers only change the model; between loop turns the driver loads what was asked: it shows
  "Loading …" first (the fetch blocks the app, not the computer), fetches the page and its style sheets, and shows the
  page, an error page, a text file as text, or "Can't show this file". A link to `#somewhere` on the same page scrolls
  there without a fetch.
- `pages.rs`: the browser's own pages (start, errors, folder listings).
- `main.rs`: the host transport: TCP through the computer's stack (`getaddrinfo`, `connect`, a 1 s receive timeout
  polled up to 15 s), files with `std::fs` (a folder: its `index.html`, else a listing).

## httpd

`httpd <port> [dir]` serves `dir` (default: the computer's root, as the village web servers do): `/` is its `index.html`
or a listing, other paths its files, with `Content-Type` for `.html .css .js .png .json .py` (else `text/plain`); paths
with `..` are refused. It prints each request (`GET /style.css`).

## Scenarios

`/ecm scenario spawn <name>` and the GameTests in `ecm_browser` run the same definitions (`testing/scenario/BrowserScenarios`).

- **`browser_local_site`**: two cabled computers. On the first, the player writes `index.html`, `style.css` (a flex bar
  with `margin-right: auto`, wrapping flex cards), `parts.html` and `plain.html` into `/home/site` (typed from Python),
  runs `gui`, opens the Browser from the launcher and types `/home/site/index.html`: the cards are side by side, the
  links pushed right, the page background is the stylesheet's. Control: `plain.html` links a missing stylesheet and still
  shows, unstyled. Back in the console (Ctrl+Alt+Backspace) `httpd 80 /home/site &`; on the second computer the Browser
  loads `10.0.60.1` styled (httpd logs `GET /style.css`), the Parts link loads `parts.html`. Control: `/nope.html` shows
  httpd's 404 text. About 5 s.
- **`browser_village`**: the nearest Tech Village's first house. The player opens the house router's and PC's screens
  (starting them), the PC leases an address and pings the Data Center, `gui`, Browser, the village site's address
  (`100.(64+n).0.10`): the window takes the site's title; the wheel scrolls to the list of the other villages; a click on
  the next village's link loads that village's site over the fiber ring. Controls: `100.(64+n).0.99` (nobody there)
  shows "Can't reach this page" after the connect timeout; `/nope.html` shows the server's 404. In a GameTest world the
  village is placed at its planned site and the ring's infrastructure started as a normal server starts it (see
  [SCENARIO_EXCEPTIONS.md](../SCENARIO_EXCEPTIONS.md)). About 21 s.
- **`browser_phone`**: an ePhone and a player-built cell network (a core with its Card Reader/Writer, a cell site, a web
  server on the core's LAN running `httpd 80 /srv/www`, as in `cell_chat`). Control first: with no modem the Browser from
  the home grid shows an error page for the server's address and keeps running. With the LTE Modem and the SIM cored
  wrote it attaches; Reload loads the page over LTE; its end is off screen until finger drags scroll there and the top
  scrolls away; a tap on the last link loads the second page. About 9 s.

The plan's `browser_phone` uses a Tech Village Phone Store phone; that needs the naturally generated world of the `tech`
client suite (`cell_tech_village`), so phase 1 runs it on a player-built network, which a one-minute GameTest can hold.

## Tests

| Layer | Command | What |
|---|---|---|
| Engine | `cargo test -p ecm-web` | URLs and form coding; the tokenizer, implied ends, entities, junk; selectors, specificity, media, shorthands; the cascade, inheritance, keywords, colours, lengths; fetch: Host, redirects and loops, cookies per host, 404, HTTP/0.9, chunked, files. `tests/layout.rs`: the village page at 400 lu (wrapping, bullets, hit-tested links), block boxes and centring, collapsing margins, flex row/justify/align/auto margins/wrap/column/reverse, longest-word minimum, a media query switching layout, a missing sheet, colours and text styles, ordered list numbering, `:hover`, junk and 1000-deep nesting never panic |
| App | `cargo test -p browser --lib` | on a mock UI connection and a scripted network: start page, typed address, error page, a clicked link, Back/Forward/Home, the server's 404, the phone layout (bottom bar, nothing past the right edge), wheel/drag/End scrolling, a local site with and without its stylesheet, folders and text files, `#anchors`, suspend and restore, hotkeys and Reload |
| Golden images | `cargo run -p browser --example record_browser_sessions -- ../src/test/resources/ui-sessions`, then `-JUnit UiGoldenTest -McVersion 26.1` | `browser_compact` (the styled sample site on a phone), `browser_medium` (flex bar and cards), `browser_wide` (a village site in the theme colours) |
| In world | `scripts\Test.ps1 -Area browser -GameTests ecm_browser` | the three scenarios |

## Limits and open items

- The fetch blocks the app while it runs (the window shows "Loading …" first); an address nobody answers takes the
  connect timeout (about 10 s). A non-blocking fetch can come with phase 3's event loop.
- No pictures yet (`img` shows its alt text), no forms, no absolute positioning, tables as flex rows: phase 2.
- No italic font; bold is drawn twice. The mono font has 16 and 32 lu only.
- Downloads, `.ecmpkg` hand-off to the App Store, bookmarks and the home page setting: phase 2.
