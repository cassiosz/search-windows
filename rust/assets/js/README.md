# assets/js: Curtain and Reader scripts

All files: plain JS, `include_str!`-able, self-contained, idempotent if injected twice, silent when
`window.chrome.webview` is absent. Ported from Search (Office Commun), MIT.

Messages out go through `window.chrome.webview.postMessage(obj)` (arrive as WebMessageReceived JSON).
Messages in are optional: `PostWebMessageAsJson` is received by `chrome.webview.addEventListener('message')`.
Every script can also just be driven with ExecuteScript.

## picker.js (Ctrl+Shift+H)
Injection: ExecuteScript on demand (or AddScriptToExecuteOnDocumentCreated; it is asleep until started).
Start: `window.__officeVeil.on()` or message `{op:"curtain.start"}`. Stop: `.off()` / `{op:"curtain.stop"}`
(Esc inside the page also stops it).
Posts:
- `{op:"curtain.pick", host, selector, label, note}` on click (picker stays on, so several can be picked)
- `{op:"curtain.off"}` when it stops (Esc, or `off()`)
- `{op:"curtain.trouble", trouble}` on failure
Expects (all optional): after a pick the app does `curtain.add_with(host, selector, label, note)`, saves, then
updates the page at once with `window.__officeVeil.css(css)` or `{op:"curtain.css", css}`.
Also exposes `peek(css, selector)` / `unpeek(css)` (used by hidden.js).

## hidden.js (Ctrl+Shift+U, "what is hidden here")
Injection: ExecuteScript on demand, after picker.js (it uses `window.__officeVeil.peek` for hover preview).
Open/refresh: `window.__officeHidden.open([{selector,label,note}])` or `{op:"curtain.list", veils:[...]}`;
`toggle(list)` and `close()` / `{op:"curtain.closeList"}` also exist. Draws a closed shadow-DOM panel.
Posts:
- `{op:"curtain.restore", host, selector}`
- `{op:"curtain.restoreAll", host}`
- `{op:"curtain.hideSomething", host}` (the app should then start the picker)
After each restore the app removes the entry from the store and re-sends the new css (`curtain.css`).

## reader.js (Ctrl+Shift+R)
Injection: ExecuteScript on demand. The ExecuteScript result is the JSON string `"read"` or `"none"`
(none = no article found; leave the page alone). Optionally set `window.__officeReaderLook = 'light'|'dark'|'system'`
first (default `system`, which follows `prefers-color-scheme`). To leave reader mode reload the page.
Posts: `{op:"reader.look", look}` when the user clicks the small Light/Auto/Dark switch (persist it if wanted),
`{op:"reader.trouble", trouble}` on error. It removes the page's own stylesheets (not `office-*` ones,
not adoptedStyleSheets such as Shield's).

## Document-start hiding
Not a file: `src/page.rs` (`start_script`) makes one AddScriptToExecuteOnDocumentCreated script per
navigation holding Shield's stylesheet and the site's hidden things, as constructed stylesheets (at document
start there is no element to put a `<style>` in). The hidden things go in a sheet of their own, one
`insertRule` per selector, kept as `window.__officeVeilSheet`; picker.js's `css`/`peek`/`unpeek` rewrite that
same sheet (making one if the page started without). `searchwin_curtain::style_script` is not used.

## Security
- Only trust `curtain.*` and `reader.*` messages from the frame/page the app injected into: check the message
  `source` URL against the tab's current URL and recompute the host with `searchwin_curtain::host_of`
  yourself; never use the `host` field in a message as the key without comparing it to the tab's host.
  Page script can call `chrome.webview.postMessage` too, so a hostile page can forge any of these messages:
  the worst a forged `curtain.pick` should do is hide something on that same site.
- Treat `selector` and `label` as untrusted strings. Reject selectors containing `{`, `}`, `;`, `/*`, `<`,
  or newlines before storing (they would break out of the one-rule-per-selector stylesheet), cap lengths
  (e.g. 500 chars), and always embed CSS in scripts through `style_script` (JSON literal), never by string concatenation.
- Never forward these messages to a different origin or act on them from subframes you did not inject into.
