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

## float-on.js / float-off.js (Ctrl+Shift+P, `src/float.rs`)
Injection: ExecuteScript on demand, in the tab's page, nothing at document start. `float-on.js` answers
`"none"` (no video playing: nothing changes) or `{floating:true, w, h}` (the video's own size, for the
little window's shape) after hiding everything but the biggest playing video and making it fill the
viewport; the page is then re-parented into the floating window (ICoreWebView2Controller::put_ParentWindow).
`float-off.js` puts the page back once it is laid out at the tab's size again (or after 400 ms / 1 s) and
leaves any picture in picture of the engine's own. Neither posts anything.

## Peek's listener (not a file: `src/peek.rs`, `start_script`)
A document-start script, given only while Settings › Peek at links is on: a trusted Shift-click (or
Alt-click) on an http(s) link in a website's top document posts `{op:"peek.open", url}`. The window hears it
only from a document that was given the script, only from a tab in the row, only while Peek is on, and only
for an http or https address.

## forms.js (sign-in offers, `src/forms.rs`)
Injection: a document-start script on every page (`AddScriptToExecuteOnDocumentCreated`, all frames); it
returns at once on anything but http(s). It posts `forms.focus` (the caret's box: rect, name step),
`forms.name` (a first step's name), `forms.sent` (a sign-in went out: the user name only) and `forms.settled`
(the boxes went away in place). It never posts a password and never receives a saved one in a message: the
app fills through `window.__officeForms.fill/name` (frozen, non-writable), which first checks the document's
own scheme and host (with its port; the app re-checks both inline before calling it), and takes the password just sent through `__officeForms.take()` (once, within 10 s).
The window hears these only from a document given the script, a frame only through the frame; a frame
of another origin than the page (scheme, host, port) is offered nothing, and saving is heard from the top document alone.
