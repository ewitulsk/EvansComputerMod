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
- [Scripts](#scripts)
- [The app](#the-app)
- [httpd](#httpd)
- [Scenarios](#scenarios)
- [Tests](#tests)
- [Limits and open items](#limits-and-open-items)

## Phases

One PR per phase.

| Phase | What | Scenario | State |
|---|---|---|---|
| 1 | `ecm-web` fetch + parse + CSS cascade + block/inline layout + flexbox; the app on desktop and phone; village sites; `file://` | `browser_village`, `browser_phone`, `browser_local_site` | **built** |
| 2 | Positioning, images, tables, forms, cookies in use; `httpd` CGI mode and virtual hosting; the Python/Lua server module; bookmarks, history and a home page setting; downloads | `browser_form` | **built** |
| 3 | JavaScript: QuickJS (rquickjs, built with wasi-sdk like Lua), the DOM subset, events, timers, `fetch()`, `localStorage` | `browser_js` | **built** |
| 4 | Names (DNS, docs/DNS.md): `http://web.v4.ring/`, `web` through the search domain, the registrar's pages at `http://nic.ring/` | `browser_dns` | **built** |

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

Desktop (medium and wide windows): `[<] [>] [Reload] [Home] [address] [Go] [Menu]` over the page, and a status line
that shows the link under the pointer or what is loading. Phone (compact): the address on top, `[<] [>] [Reload] [Home]
[Menu]` in a bar at the bottom, in thumb reach. **Menu**: Bookmark this page, Bookmarks (`about:bookmarks`), History
(`about:history`, the last 100 pages), Set as home page. They are saved in the app's data folder
(`/var/apps/dev.ecm.browser/browser.json`). Until a home page is set, Home is the computer's own village site when it sits
on a village cable or data center LAN (`100.(64+n).0-1.x`), else the start page.

Forms: click a field and type (Left/Right/Home/End, Backspace/Delete, Tab to the next field, Escape to leave it); Enter
sends the form (a new line in a textarea); a checkbox or radio button flips on a click; a select opens a menu of its
options; a submit button sends. GET forms go to the address with the query, POST forms send an urlencoded body; the
browser follows the server's redirect (303 after a POST) and keeps the cookies it sets, per host, for the session.
Required fields are the server's business: the browser sends what is there.

Pictures (`<img>`, `background-image`) are fetched after the page is shown, one per turn, decoded (PNG: 8-bit grey,
grey+alpha, RGB, RGBA, palette; up to 1024 px a side) and uploaded as image resources. A picture that doesn't load shows
its `alt` text in a frame.

Anything a server sends that the Browser can't show (not HTML, text, CSS or PNG; or `Content-Disposition: attachment`)
is a download: saved in `/home/Downloads/` and shown as a page with links to it. An app package (`.ecmpkg`) says how to
install it (`pkg install /home/Downloads/<name>.ecmpkg`); the App Store has no "install from a file" yet.

Input: click or tap a link; the wheel, a drag (a finger on the phone), arrows, Page Up/Down, Space, Home and End scroll;
Alt+Left / Alt+Right or Backspace go back and forward; F5 reloads; Ctrl+L goes to the address bar. In hand on an ePhone,
hotkey slot 1 is Back, 2 scrolls up a page, 3 down. A suspend saves the history and the scroll position; the page opens
again after a cold start.

Names resolve through the computer's stub resolver (`/etc/hosts`, then `/etc/resolv.conf`'s servers and search
domains, docs/DNS.md): `web.v4.ring` is another village's site, and in a Tech Village a single word such as `web` is
completed with the village's search domain (`web.v1.ring`). The address bar keeps the name typed.

Errors are pages: "Can't reach this page" (nobody answered), "Can't find the server" (a name that doesn't resolve, with
the resolver's reason: "name not found", "no answer from 100.65.0.53", "no DNS server configured"), "No
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
| Media | `img src alt width height` (PNG; the alt text in a frame until it loads, or when it can't) |
| Forms | `form action method`, `input` text/password/email/number/search/hidden/checkbox/radio/submit/image/button/reset, `textarea`, `select`/`option`, `button`, `label` (`for`, or wrapping its control), `fieldset disabled`, `maxlength`, `formaction`/`formmethod` |
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
- **Layout**: `display: block|inline|inline-block|none|flex|inline-flex|list-item|table|table-row-group|table-row|
  table-cell|table-caption` (`grid` is a block); tables: columns sized from their cells' minimum and preferred widths
  (shrink-to-fit unless the table has a width), `border-spacing`, `border-collapse: collapse` (no spacing),
  `vertical-align` top/middle/bottom in cells, captions, row backgrounds; no `colspan`/`rowspan` in v1; flexbox: `flex-direction` (and reverse), `flex-wrap`, `justify-content`,
  `align-items`, `align-self`, `gap`/`row-gap`/`column-gap`, `flex-grow`/`-shrink`/`-basis` and the `flex` and
  `flex-flow` shorthands, `order`, auto margins on items, `min-width: auto` (an item isn't squeezed below its longest
  word); `position: relative | absolute | fixed` with `top`/`right`/`bottom`/`left` (`inset`) and `z-index`: absolute
  boxes leave the flow and are placed against their nearest positioned ancestor's padding box (the viewport when there
  is none; both insets of an axis stretch the box), painted over the flow in z-index order; fixed boxes are placed
  against the viewport and stay on screen while the page scrolls. `background-image: url()` (PNG, drawn once at the
  padding box's corner); inline blocks and form controls sit on their text's baseline.
- **Units**: `px % em rem vw vh vmin vmax pt pc in cm mm ch ex`, `calc()` with `+` and `-`. **Colours**: names, `#rgb`,
  `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`/`rgba()` (comma or space syntax), `hsl()`/`hsla()`, `transparent`,
  `currentcolor`.
- **Theme**: unstyled pages use the shell theme's colours (text, background, accent for links, borders, control
  surfaces), so they look native in light and dark; a theme change re-renders the page.
- Not in v1: grid, floats (blocks), transforms, transitions and animations, custom fonts, `::before`/`::after`.

The mono font exists at 16 and 32 lu only: preformatted text (`pre`, `white-space: pre`) uses it at 16 or 32; small
inline `code` keeps the ui font at the paragraph's size. Characters the Latin-1 fonts don't have become their nearest
text (curly quotes, dashes, `…`, arrows, `•` as `·`).

## Scripts

Pages run JavaScript (`<script>` inline and `<script src>`, in document order) on QuickJS: `rust/crates/ecm-js`, the
`rquickjs` crate, whose C sources are built for `wasm32-wasip1` with wasi-sdk's clang like Lua's (`WASI_SDK_PATH`;
`scripts/stage-wasm.sh` skips `browser` and `lua` without it). One runtime and context per page, inside the Browser's
process; it ends when the page goes away. It is a page runtime, not a scripting package, so the Python/Lua parity rule
doesn't apply.

- **Budget**: every entry into the page's scripts (loading them, an event, a timer tick) runs under a time budget,
  250 ms (1 s while the page's scripts first run). QuickJS's interrupt handler stops a script that runs longer
  (`while (true) {}`); the console says `[stopped] a script ran too long and was stopped`, the context stays usable and
  the Browser and the computer never hang. Memory is limited to 16 MiB, the JS stack to 256 KiB.
- **DOM**: `document.getElementById`, `querySelector`/`querySelectorAll` (the CSS selectors above), `getElementsBy...`,
  `createElement`, `createTextNode`, `body`, `head`, `title`; elements: `textContent`, `innerHTML`/`outerHTML` (through the
  page's parser), `innerText`, `value`, `checked`, `disabled`, `hidden`, `id`, `className`, `classList`
  (`add`/`remove`/`toggle`/`contains`), `style.x` (`fontSize` → `font-size` in the `style` attribute), `dataset`,
  `get/set/remove/hasAttribute`, `appendChild`, `insertBefore`, `removeChild`, `replaceChild`, `append`, `prepend`,
  `before`, `after`, `remove`, `children`, `childNodes`, `parentNode`, sibling and child accessors, `matches`,
  `closest`, `click()`. A document changed by a script is styled, laid out and drawn again once, after the script.
- **Events**: `addEventListener`/`removeEventListener` and `on<type>` properties for `click`, `input`, `change`,
  `submit`, `keydown` (`e.key`), `DOMContentLoaded`, `load`; they bubble from the target to the document and the window;
  `preventDefault()` stops the default (following a link, sending a form, Enter sending it), `stopPropagation()`.
- **Timers**: `setTimeout`, `setInterval`, `clearTimeout`/`clearInterval`, `requestAnimationFrame` (about 60 a second),
  `performance.now()`, `queueMicrotask`. The Browser wakes for the next one.
- **Network and storage**: `fetch(url, {method, body})` over the computer's network (and `file://`), resolved against the
  page, with the session's cookies: a Promise of a response with `ok`, `status`, `url`, `headers.get`, `text()`,
  `json()`; a failure rejects with a `TypeError`. `localStorage` (and `sessionStorage`, the same) per host (and one for
  `file:` pages), kept in the app's data folder (`storage/<host>.json`).
- **Window**: `location.href` (setting it navigates), `assign`, `reload`, `pathname`, `search`, `hash`, `host`;
  `history.back()`/`forward()`; `console.log`/`info`/`warn`/`error` into the Browser's console (Menu > Console);
  `alert` and `confirm` show a dialog in the window and wait for the player's answer (OK / Cancel), as in a browser;
  `innerWidth`/`innerHeight`, `navigator.userAgent`.
- Not in v1: canvas 2D, WebSockets, workers, ES modules (a `type=module` script runs as a classic one), `prompt`
  (returns null).

The Rust side is a set of native functions (`__n.*` in `ecm-js/src/lib.rs`) on the document, the network, storage and
the dialogs; `prelude.js` builds the web API on them. The page lives in the Browser's model and is moved into the
script host only while a script runs.

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

`httpd <port> [dir] [--cgi <prefix>]` (`rust/wasm-programs/httpd`, its pure parts in `lib.rs`, tested on the host):

- `dir`: the folder served (default: the computer's root, as the village web servers do). `/` and folders are their
  `index.html` or a listing; files get `Content-Type` by extension (`.html .css .js .png .json .py .lua`, else
  `text/plain`); paths with `..` (also `%2e%2e`) are refused. It prints each request (`GET /style.css`).
- **Virtual hosting**: a request whose `Host:` names a folder in `dir` is served from it: with
  `/srv/www/web.v4.ring/index.html`, `http://web.v4.ring/` gets that page, other names get `dir`'s.
- **CGI**: with `--cgi /cgi`, a request under the prefix runs the script it names, `.py` with `python` and `.lua` with
  `lua`, in a command session of its own (`ipc_spawn_command`; a wide terminal so lines aren't wrapped). The script's
  stdin is the request: `REQUEST_METHOD`, `PATH_INFO`, `QUERY_STRING`, `CONTENT_TYPE`, `CONTENT_LENGTH`, `REMOTE_ADDR`,
  `HTTP_HOST`, `HTTP_COOKIE`, `HTTP_USER_AGENT` as `NAME=value` lines, a blank line, the body. What it prints is the
  response: optional `Status:`, `Content-Type:`, `Location:`, `Set-Cookie:` header lines, a blank line, the page (no
  header block: the whole output is HTML; a `Location:` without a status is a 302). A script has 10 s
  (`504 Gateway Timeout`); a missing script is a 404, another extension a 403. httpd prints `  /cgi/x.py -> 303`.

## Server module: web

The same module in Python (`import web`, `rust/wasm-programs/python/src/web.py`) and Lua (`local web = require("web")`,
`rust/wasm-programs/lua/src/lua/web.lua`), written on top of each language's `net` module:

| Name | What |
|---|---|
| `web.serve(port, handler)` | a server of its own: answers HTTP requests on `port` one at a time until Ctrl+T, printing `METHOD /path status` |
| `web.cgi(handler)` | answers the one request `httpd --cgi` hands the script, on stdout |
| `web.handle(raw, handler)` | one raw HTTP request through the handler: the response's bytes (what `serve` does per connection; tests) |
| `web.Request(method, target, headers, body, remote)` | `method`, `path`, `query`, `form` (a POSTed urlencoded form), `headers` (lower-case), `cookies`, `body`, `remote` |
| `web.Response(body, status, headers, content_type)` | `set_cookie(name, value)` (Lua: `resp:set_cookie`) |
| `web.redirect(location, status=303)` | a redirect (after a POST, so the browser GETs the page) |
| `web.escape(text)`, `web.parse_query(s)` | HTML escaping; `a=1&b=x+y` to a dict / table |
| `web.parse_cgi(text)`, `web.cgi_output(resp)` | the CGI protocol's two halves |
| `web.REASONS`, `web.MAX_BODY` | status reasons; the largest request body read (64 KiB) |

A handler gets a request and returns a string (an HTML page), a response, or None/nil (404). An exception or Lua error
becomes a 500 page with its message.

```python
import web
def hello(req):
    if req.method == "POST" and not req.form.get("name"):
        return web.Response("<p>Name required</p>", 400)
    return "<h1>Hello, " + web.escape(req.form.get("name", req.query.get("name", "world"))) + "</h1>"
web.cgi(hello)            # as /srv/www/cgi/hello.py under: httpd 80 /srv/www --cgi /cgi
```

```lua
local web = require("web")
web.serve(8080, function(req)
  return "<h1>Hello, " .. web.escape(req.query.name or "world") .. "</h1>"
end)
```

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
- **`browser_form`** (phase 2): two cabled computers. The server writes a front page, a Python CGI guestbook
  (`/srv/www/cgi/guestbook.py`, `web.cgi`) and a Lua guestbook (`/srv/guestbook.lua`, `web.serve(8080, ...)`), runs
  `httpd 80 /srv/www --cgi /cgi &` and `lua /srv/guestbook.lua &`. On the other computer the Browser opens the front page
  and the Python guestbook. Control: Sign with nothing filled in: the script answers 400 and the form comes back with
  "Name and message are required.". Then the player clicks the name field, types, clicks the message field, types and
  signs: the script saves the entry and answers 303, the Browser follows it and the table has the entry; the page says
  "Welcome back, Pat" from the `who` cookie. Back (Alt+Left), the Lua guestbook on port 8080: it greets Pat too (the
  cookie is the host's), refuses an empty message, and Enter in the message field signs it. About 14 s.
- **`browser_js`** (phase 3): the server writes a page, its script `app.js` and `news.json` into `/srv/js` and serves
  them with `httpd 80 /srv/js`. The Browser on the other computer loads the page and its script (httpd logs `GET
  /app.js`); three clicks on Add one count 1, 2, 3; Load news `fetch()`es `news.json` and shows its headline (httpd logs
  `GET /news.json`). Control: Freeze runs `while (true) {}`; a click on Add one right after counts 4 within a second
  (the budget stopped the loop), and Menu > Console shows the counter's `console.log` lines and `[stopped] a script ran
  too long and was stopped`. About 11 s.
- **`browser_dns`** (phase 4): at a Tech Village house PC (the ring scenarios' start, docs/DNS.md) after its
  `/etc/resolv.conf` came from DHCP: the Browser loads `web.vN.ring` (another village's site, by name) and `web` (its own,
  through the search domain). Control: `nosuch.ring` is an error page, "Can't find the server ... name not found". At
  `http://nic.ring/` the player searches a fresh name (free), creates an account (the registrar logs the Browser in with a
  session cookie), registers `NAME.ecm`, and on the home page's record form for it points `www` (type A, filled in) at
  the village's web server `100.(64+n).0.10`; `http://www.NAME.ecm/` then shows the village's site. Control: registering
  the same name again: "That didn't work ... already registered". About 40 s once the ring is up.
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
| Engine, phase 2 | `cargo test -p ecm-web` (`tests/phase2.rs`, `forms.rs`, `png.rs`) | absolute boxes against a relative ancestor and the viewport, out of the flow; z-order; fixed boxes in their own layer and hit-tested in viewport coordinates; table columns from their cells, spacing, a given width, middle alignment, the longest-word minimum; pictures and backgrounds by address, scaled with their shape, alt text when missing; a focused field's caret and moved text; form submissions (POST body in document order, CRLF in textareas, disabled and unpressed buttons left out), toggles, radio groups, selects, labels, Tab order; PNG round trip |
| App | `cargo test -p browser --lib` | on a mock UI connection and a scripted network: start page, typed address, error page, a clicked link, Back/Forward/Home, the server's 404, the phone layout (bottom bar, nothing past the right edge), wheel/drag/End scrolling, a local site with and without its stylesheet, folders and text files, `#anchors`, suspend and restore, hotkeys and Reload; phase 2: typing into a form and POSTing it (Backspace edits the field, Enter makes a new line in a textarea, a checkbox, the select's popup, the cookie back on the redirect), Enter sends a GET form, pictures fetched and uploaded (and a missing one remembered), bookmarks/home/history saved and loaded again, downloads, a click right after typing in the same batch of events |
| Scripts | `cargo test -p ecm-js` | QuickJS with the prelude against ecm-web documents: queries, creating and moving nodes, `innerHTML`, `classList`, `style`, `value`, the same object for the same element, `title`; page scripts in order with `DOMContentLoaded`/`load` (an error in one doesn't stop the next); clicks bubbling and `preventDefault`; timers, intervals and frames; `while (true) {}` in a listener and in a timer stopped by the budget with the context still working; `fetch()` (relative, POST, `json()`, a rejection), `localStorage`, `location`, `history`, `confirm`/`alert`, `console`; key events |
| App, phase 3 | `cargo test -p browser --lib` | a counter, a `fetch()`, a runaway click stopped while the page still counts, a key listener; scripts stopping a link and a form (and Enter sending it when allowed); a timer firing, `localStorage` kept for the next Browser, `confirm`; the dialog waiting for the player's answer |
| Servers | `cargo test -p httpd --lib`, `cargo test -p python --lib web_module`, `cargo test -p lua --lib` | httpd's command line, virtual hosts, paths (`..` refused), the CGI input and output; `web` in RustPython and in Lua with the same handler: query, form, cookies, 303 + Set-Cookie, 400, 500, 404, junk, the CGI protocol; the Python/Lua parity test includes `web` |
| Golden images | `cargo run -p browser --example record_browser_sessions -- ../src/test/resources/ui-sessions`, then `-JUnit UiGoldenTest -McVersion 26.1` | `browser_compact` (the styled sample site on a phone), `browser_medium` (flex bar and cards), `browser_wide` (a village site in the theme colours), `browser_forms` (a fixed bar, a picture with a positioned badge, a table, a form with a typed-into field) |
| Registrar pages | `cargo test -p ecm-dns registrar_web` | the session cookie (start, read from a `Cookie:` header, log out), the home page logged out (search, create, log in) and in (register, the account's domains with a record form each), result pages in words, escaping |
| In world | `scripts\Test.ps1 -Area browser -GameTests ecm_browser` | the six scenarios (`browser_dns` places Tech Village 1 and boots the ring, as `DnsTests.ring`) |

## Limits and open items

- The fetch blocks the app while it runs (the window shows "Loading …" first); an address nobody answers takes the
  connect timeout (about 10 s). A non-blocking fetch can come with phase 3's event loop.
- No italic font; bold is drawn twice. The mono font has 16 and 32 lu only.
- Tables: no `colspan`/`rowspan`. Backgrounds: no repeat, position or size. Pictures: PNG only.
- `.ecmpkg` downloads: the App Store can't install from a file yet, so the page shows the `pkg install` command.
- A CGI script starts a new interpreter per request: about 2 s for Python, less for Lua. `web.serve` keeps one running.
- Picture resources stay uploaded for the Browser's life (cached by address).
- Size: `browser.wasm` must stay under the runtime's 2 MiB compile-before-run limit (`ChicoryRuntime`); a bigger module
  starts interpreted and an open Browser stays so, far too slow to type into a page. `rust/Cargo.toml` builds it (and
  ecm-web, ecm-js, QuickJS, ecm-ui) for size and strips its symbols (about 1.7 MB); `scripts/stage-wasm.sh` stops with an
  error if it grows past the limit.
- Scripts run inside the Browser's loop: a long-running fetch blocks the page (fetch is synchronous inside, its Promise
  already settled when it returns); key events are seen by scripts after the Browser handled the key (only Enter's form
  sending can be prevented).
