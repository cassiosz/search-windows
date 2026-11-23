//! The pages: one WebView2 controller per tab, in the window's card.
//!
//! The environment (the browser process) is started at launch, before the
//! window, because that is most of what a launch costs. A tab's controller is
//! attached only when that tab is first shown — a tab restored from last
//! session costs nothing until it is clicked — and dropped again when the tab
//! sleeps or closes, which gives its renderer's memory back.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::Win32::Foundation::{E_POINTER, HGLOBAL, HWND, RECT};
use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
use windows::Win32::System::Com::{CoTaskMemFree, STREAM_SEEK_SET};
use windows_core::{BOOL, HSTRING, Interface, PCWSTR, PWSTR, Result, w};

pub type TabId = u64;

/// The tab a page's events are told for. Shared by every handler of the
/// page, so a peek kept as a tab (rename) is heard as that tab from then on.
type Key = Rc<Cell<TabId>>;

/// Peeks' pages are numbered down from the top, where no tab ever gets to.
const PEEK_BASE: TabId = u64::MAX;

/// What a page tells the window about itself.
pub enum Event {
    Started,
    Moved(String),
    Title(String),
    /// Done, and whether the page came (an error page is false).
    Loaded(bool),
    /// A page asked for a new window: it becomes a tab, `background` when Ctrl was held.
    NewWindow { url: String, background: bool, popup: bool },
    /// Back or forward became possible, or stopped being.
    History,
    /// The site's icon, as a PNG, for the page now in the tab.
    Icon(Vec<u8>),
    /// The page started or stopped playing sound.
    Audible(bool),
    /// The page took the keyboard (a click in it): in a split, its pane is the one in front.
    Focused,
}

#[derive(Default)]
struct View {
    controller: Option<ICoreWebView2Controller>,
    view: Option<ICoreWebView2>,
    pending: Option<String>,
    attaching: bool,
    /// Where the page is, as it last said; Shield judges third parties against it.
    url: Option<String>,
    /// Where the page is going: that request is the navigation itself, never blocked.
    heading_to: Option<String>,
    /// The scripts run as each document is made: Shield's stylesheet for the
    /// site (SHIELD), and what you hid, on every site (VEILS).
    start: [StartScript; 4],
    /// Which of our scripts this document was given (crate::veil's *_IN):
    /// a web page is listened to only for what those scripts say.
    injected: u8,
    /// The picker is pointing: only now does a pick count.
    picking: bool,
    key: Option<Key>,
}

const SHIELD: usize = 0;
const VEILS: usize = 1;
/// Peek's click listener, only while Peek is on (crate::peek).
const PEEK: usize = 2;
/// The sign-in boxes' watcher (crate::forms), on every web page.
const FORMS: usize = 3;

/// A document-start script: what it says, and WebView2's id for it once added.
#[derive(Default)]
struct StartScript {
    text: String,
    id: Option<String>,
}

#[derive(Default)]
struct Pages {
    environment: Option<ICoreWebView2Environment>,
    window: HWND,
    views: HashMap<TabId, View>,
    front: Option<TabId>,
    /// The card: where the page in front goes when it stands alone.
    bounds: RECT,
    /// Split View: every page on screen and its place. Empty when the page
    /// in front stands alone, which is all there is with Split View off.
    panes: Vec<(TabId, RECT)>,
    /// Bumped when a whole row of tabs is put away (another space): a page
    /// still being made for the row that left is closed as it arrives, never
    /// handed to the tab with the same number in the row that came.
    epoch: u64,
    background: (u8, u8, u8),
    /// Pages out of the card, each in a window of its own at a place in it:
    /// the floating video's, a peek's. Nothing the card does moves or hides them.
    elsewhere: HashMap<TabId, RECT>,
    /// Peeks made so far, for numbering the next.
    peeks: u64,
}

impl Pages {
    fn rect_for(&self, id: TabId) -> RECT {
        if let Some(rect) = self.elsewhere.get(&id) {
            return *rect;
        }
        self.panes.iter().find(|(p, _)| *p == id).map_or(self.bounds, |(_, r)| *r)
    }

    /// In the card, or beside it in a split.
    fn on_screen(&self, id: TabId) -> bool {
        !self.elsewhere.contains_key(&id) && (self.front == Some(id) || self.panes.iter().any(|(p, _)| *p == id))
    }

    fn shown_controllers(&self) -> Vec<ICoreWebView2Controller> {
        self.views.iter().filter(|(id, _)| self.on_screen(**id)).filter_map(|(_, v)| v.controller.clone()).collect()
    }
}

thread_local! {
    static PAGES: RefCell<Pages> = RefCell::new(Pages::default());
}

/// Never hold the borrow across a call into WebView2: its events can arrive inside it.
fn with<R>(f: impl FnOnce(&mut Pages) -> R) -> R {
    PAGES.with(|pages| f(&mut pages.borrow_mut()))
}

pub fn warm(window: HWND) {
    with(|p| p.window = window);
    let folder = HSTRING::from(crate::data_folder().join("Pages").as_os_str());
    let options = CoreWebView2EnvironmentOptions::default();
    // Nothing leaves the PC but the pages asked for: no SmartScreen lookups.
    unsafe { options.set_additional_browser_arguments("--disable-features=msSmartScreenProtection".to_string()) };
    // search://settings and the like: a secure origin of their own, with a
    // host (settings, history), that no other origin may fetch from.
    let scheme = CoreWebView2CustomSchemeRegistration::new(crate::omnibox::INTERNAL_SCHEME.to_string());
    unsafe {
        scheme.set_treat_as_secure(true);
        scheme.set_has_authority_component(true);
        options.set_scheme_registrations(vec![Some(scheme.into())]);
    }
    let options: ICoreWebView2EnvironmentOptions = options.into();
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(|result, environment| {
        result?;
        let waiting: Vec<TabId> = with(|p| {
            p.environment = environment;
            p.views.iter().filter(|(_, v)| v.pending.is_some()).map(|(id, _)| *id).collect()
        });
        for id in waiting {
            attach(id);
        }
        Ok(())
    }));
    let started = unsafe { CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), &folder, &options, &handler) };
    if let Err(error) = started {
        crate::crash_log(&format!("page engine did not start: {error}"));
    }
}

fn attach(id: TabId) {
    let (environment, window, epoch) = with(|p| {
        let ready = p.environment.clone();
        let window = p.window;
        let view = p.views.entry(id).or_default();
        if view.attaching || view.controller.is_some() || ready.is_none() {
            return (None, window, p.epoch);
        }
        view.attaching = true;
        (ready, window, p.epoch)
    });
    let Some(environment) = environment else { return };
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |result, controller| {
        result?;
        configure(id, epoch, controller.ok_or_else(|| windows_core::Error::from(E_POINTER))?)
    }));
    // A private tab's page is InPrivate, in a profile of its own: its cookies
    // and storage are kept in memory and go when its last page closes. An
    // engine too old for that gets no page at all, never a page that keeps things.
    // Any other tab is in its space's store: the default one, the store there
    // always was, unless the space or its profile has cookies of its own.
    let made = match (crate::private_profile(id), crate::spaces::store()) {
        (None, None) => unsafe { environment.CreateCoreWebView2Controller(window, &handler) },
        (private, store) => unsafe {
            environment.cast::<ICoreWebView2Environment10>().and_then(|environment| {
                let options = environment.CreateCoreWebView2ControllerOptions()?;
                let in_private = private.is_some();
                options.SetProfileName(&HSTRING::from(private.or(store).unwrap_or_default()))?;
                options.SetIsInPrivateModeEnabled(in_private)?;
                environment.CreateCoreWebView2ControllerWithOptions(window, &options, &handler)
            })
        },
    };
    if let Err(error) = made {
        with(|p| p.views.entry(id).or_default().attaching = false);
        crate::crash_log(&format!("page did not attach: {error}"));
    }
}

fn configure(id: TabId, epoch: u64, controller: ICoreWebView2Controller) -> Result<()> {
    // The tab may have closed while its page was being made, or its whole row gone.
    // A peek is shown, in its panel, and takes the keys, as the tab in front does.
    let (wanted, bounds, background, front, shown) = with(|p| {
        let away = p.elsewhere.contains_key(&id);
        (p.views.contains_key(&id) && p.epoch == epoch, p.rect_for(id), p.background, p.front == Some(id) || away, p.on_screen(id) || away)
    });
    if !wanted {
        unsafe { controller.Close()? };
        return Ok(());
    }
    let key: Key = Rc::new(Cell::new(id));
    unsafe {
        let view = controller.CoreWebView2()?;
        let settings = view.Settings()?;
        settings.SetIsStatusBarEnabled(false)?;
        if let Ok(more) = settings.cast::<ICoreWebView2Settings4>() {
            more.SetIsPasswordAutosaveEnabled(false)?;
            more.SetIsGeneralAutofillEnabled(false)?;
        }
        set_background_of(&controller, background);

        let mut token = Default::default();
        guard(key.clone(), &view, &mut token)?;
        let k = key.clone();
        view.add_SourceChanged(
            &SourceChangedEventHandler::create(Box::new(move |sender, _| {
                let id = k.get();
                if let Some(view) = sender {
                    let mut text = PWSTR::null();
                    view.Source(&mut text)?;
                    let url = take(text);
                    with(|p| {
                        if let Some(slot) = p.views.get_mut(&id) {
                            slot.url = Some(url.clone());
                        }
                    });
                    crate::page_event(id, Event::Moved(url));
                }
                Ok(())
            })),
            &mut token,
        )?;
        let k = key.clone();
        view.add_DocumentTitleChanged(
            &DocumentTitleChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(view) = sender {
                    let mut title = PWSTR::null();
                    view.DocumentTitle(&mut title)?;
                    crate::page_event(k.get(), Event::Title(take(title)));
                }
                Ok(())
            })),
            &mut token,
        )?;
        let k = key.clone();
        view.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |sender, args| {
                let id = k.get();
                if let (Some(view), Some(args)) = (sender, args) {
                    let mut uri = PWSTR::null();
                    args.Uri(&mut uri)?;
                    let uri = take(uri);
                    let script = shield_script(&uri);
                    with(|p| {
                        if let Some(slot) = p.views.get_mut(&id) {
                            slot.heading_to = Some(uri);
                        }
                    });
                    set_start(id, &view, SHIELD, script);
                }
                crate::page_event(id, Event::Started);
                Ok(())
            })),
            &mut token,
        )?;
        // A new document has none of the scripts the last one was given, so
        // nothing it posts is listened to until we give it one again.
        // Peek's listener is a document-start script, so a new document has
        // it from the start, while Peek is on.
        let k = key.clone();
        view.add_ContentLoading(
            &ContentLoadingEventHandler::create(Box::new(move |_, _| {
                with(|p| {
                    if let Some(slot) = p.views.get_mut(&k.get()) {
                        let peek = if slot.start[PEEK].text.is_empty() { 0 } else { crate::peek::PEEK_IN };
                        let forms = if slot.start[FORMS].text.is_empty() { 0 } else { crate::forms::FORMS_IN };
                        slot.injected = peek | forms;
                        slot.picking = false;
                    }
                });
                Ok(())
            })),
            &mut token,
        )?;
        let k = key.clone();
        view.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                let mut ok = BOOL::default();
                if let Some(args) = args {
                    args.IsSuccess(&mut ok)?;
                }
                crate::page_event(k.get(), Event::Loaded(ok.as_bool()));
                Ok(())
            })),
            &mut token,
        )?;
        let k = key.clone();
        view.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                let id = k.get();
                if let Some(args) = args {
                    let mut uri = PWSTR::null();
                    args.Uri(&mut uri)?;
                    args.SetHandled(true)?;
                    let background = crate::held(windows::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL);
                    // A window the page sizes is a pop-up of its own (a sign-in, a
                    // payment): it stays with its page, never sent to another space.
                    let mut sized = BOOL::default();
                    let popup = args.WindowFeatures().and_then(|f| f.HasSize(&mut sized)).is_ok() && sized.as_bool();
                    crate::page_event(id, Event::NewWindow { url: take(uri), background, popup });
                }
                Ok(())
            })),
            &mut token,
        )?;
        // What our own pages ask of the window. The sender is judged by the
        // address of the page that posted, never by what the message says.
        let k = key.clone();
        view.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |sender, args| {
                let id = k.get();
                let (Some(view), Some(args)) = (sender, args) else { return Ok(()) };
                let mut source = PWSTR::null();
                args.Source(&mut source)?;
                let source = take(source);
                if !crate::internal::is_internal(&source) {
                    // A web page is heard only after we put a script of ours in
                    // it, only from the document it is in now, and only for what
                    // that script says (veil::message). Any page can post the
                    // same words; the rest is ignored.
                    let injected = with(|p| p.views.get(&id).map_or(0, |v| v.injected));
                    if injected == 0 {
                        return Ok(());
                    }
                    let mut now = PWSTR::null();
                    view.Source(&mut now)?;
                    if take(now) != source {
                        return Ok(());
                    }
                    let mut json = PWSTR::null();
                    args.WebMessageAsJson(&mut json)?;
                    let json = take(json);
                    if injected & crate::peek::PEEK_IN != 0 && crate::peek::message(id, &json) {
                        return Ok(());
                    }
                    if injected & crate::forms::FORMS_IN != 0 && crate::forms::message(id, None, &source, &json) {
                        return Ok(());
                    }
                    crate::veil::message(id, &source, &json, injected);
                    return Ok(());
                }
                let mut json = PWSTR::null();
                args.WebMessageAsJson(&mut json)?;
                let Some(answer) = crate::internal::message(id, &source, &take(json)) else { return Ok(()) };
                // Gone on to a web page while it was being answered: that page gets nothing.
                let mut now = PWSTR::null();
                view.Source(&mut now)?;
                if crate::internal::is_internal(&take(now)) {
                    view.PostWebMessageAsJson(&HSTRING::from(answer))?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        // A frame's messages come through the frame itself, never the page:
        // only the sign-in watcher is heard from one, and crate::forms
        // judges the frame's site against the page's (vault::offers).
        if let Ok(framed) = view.cast::<ICoreWebView2_4>() {
            let k = key.clone();
            framed.add_FrameCreated(
                &FrameCreatedEventHandler::create(Box::new(move |_, args| {
                    let Some(frame) = args.and_then(|a| a.Frame().ok()).and_then(|f| f.cast::<ICoreWebView2Frame2>().ok()) else {
                        return Ok(());
                    };
                    let k = k.clone();
                    let mut token = 0;
                    frame.add_WebMessageReceived(
                        &FrameWebMessageReceivedEventHandler::create(Box::new(move |sender, args| {
                            let id = k.get();
                            let (Some(sender), Some(args)) = (sender, args) else { return Ok(()) };
                            if with(|p| p.views.get(&id).map_or(0, |v| v.injected)) & crate::forms::FORMS_IN == 0 {
                                return Ok(());
                            }
                            let (mut source, mut json) = (PWSTR::null(), PWSTR::null());
                            args.Source(&mut source)?;
                            args.WebMessageAsJson(&mut json)?;
                            let (source, json) = (take(source), take(json));
                            crate::forms::message(id, sender.cast::<ICoreWebView2Frame2>().ok(), &source, &json);
                            Ok(())
                        })),
                        &mut token,
                    )?;
                    Ok(())
                })),
                &mut token,
            )?;
        }
        let k = key.clone();
        view.add_HistoryChanged(
            &HistoryChangedEventHandler::create(Box::new(move |_, _| {
                crate::page_event(k.get(), Event::History);
                Ok(())
            })),
            &mut token,
        )?;
        // A page playing sound is kept awake (tabs' "playing sound" rule).
        if let Ok(sounding) = view.cast::<ICoreWebView2_8>() {
            let k = key.clone();
            sounding.add_IsDocumentPlayingAudioChanged(
                &IsDocumentPlayingAudioChangedEventHandler::create(Box::new(move |sender, _| {
                    let Some(view) = sender.and_then(|v| v.cast::<ICoreWebView2_8>().ok()) else { return Ok(()) };
                    let mut playing = BOOL::default();
                    view.IsDocumentPlayingAudio(&mut playing)?;
                    crate::page_event(k.get(), Event::Audible(playing.as_bool()));
                    Ok(())
                })),
                &mut token,
            )?;
        }
        // The site's icon, asked for only when the page says it has one: a tab
        // that is never shown never costs a decode.
        if let Ok(iconic) = view.cast::<ICoreWebView2_15>() {
            let k = key.clone();
            iconic.add_FaviconChanged(
                &FaviconChangedEventHandler::create(Box::new(move |sender, _| {
                    let id = k.get();
                    let Some(view) = sender.and_then(|v| v.cast::<ICoreWebView2_15>().ok()) else { return Ok(()) };
                    let mut uri = PWSTR::null();
                    view.FaviconUri(&mut uri)?;
                    if take(uri).is_empty() {
                        return Ok(());
                    }
                    let _ = view.GetFavicon(
                        COREWEBVIEW2_FAVICON_IMAGE_FORMAT_PNG,
                        &GetFaviconCompletedHandler::create(Box::new(move |result, stream| {
                            result?;
                            if let Some(png) = stream.and_then(|s| read_all(&s)) {
                                crate::page_event(id, Event::Icon(png));
                            }
                            Ok(())
                        })),
                    );
                    Ok(())
                })),
                &mut token,
            )?;
        }
        // Downloads go where crate::downloads says, without WebView2's own bubble.
        if let Ok(downloading) = view.cast::<ICoreWebView2_4>() {
            let k = key.clone();
            downloading.add_DownloadStarting(
                &DownloadStartingEventHandler::create(Box::new(move |_, args| {
                    match args {
                        Some(args) => crate::downloads::starting(k.get(), &args),
                        None => Ok(()),
                    }
                })),
                &mut token,
            )?;
        }
        // Keys pressed in the page go to the page's process; the window's own shortcuts come back here.
        // Esc in the floating video's page is the floating window's.
        let k = key.clone();
        controller.add_AcceleratorKeyPressed(
            &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
                    args.KeyEventKind(&mut kind)?;
                    if kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN || kind == COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {
                        let mut key = 0u32;
                        args.VirtualKey(&mut key)?;
                        let escape = key == u32::from(windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE.0);
                        if (escape && crate::float::escape(k.get())) || crate::shortcut(key) {
                            args.SetHandled(true)?;
                        }
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        // A click in a page beside another (Split View) makes it the one in front.
        let k = key.clone();
        controller.add_GotFocus(
            &FocusChangedEventHandler::create(Box::new(move |_, _| {
                crate::page_event(k.get(), Event::Focused);
                Ok(())
            })),
            &mut token,
        )?;

        controller.SetBounds(bounds)?;
        controller.SetIsVisible(shown)?;
        let pending = with(|p| {
            let slot = p.views.entry(id).or_default();
            slot.controller = Some(controller.clone());
            slot.view = Some(view.clone());
            slot.attaching = false;
            slot.key = Some(key.clone());
            slot.pending.take()
        });
        // Set before the first page is asked for, so they are in place for it:
        // set as a navigation starts, a script can be late for that navigation.
        set_start(id, &view, VEILS, crate::veil::start_script());
        set_start(id, &view, PEEK, crate::peek::start_script());
        set_start(id, &view, FORMS, crate::forms::start_script());
        if let Some(url) = pending {
            set_start(id, &view, SHIELD, shield_script(&url));
            view.Navigate(&HSTRING::from(url))?;
            if front {
                controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC)?;
            }
        }
    }
    Ok(())
}

/// Shield, on Windows. WebView2 is told the blocked domains as request filters,
/// so only requests to those domains ever cross into this process — every other
/// request costs nothing. Each one that does is judged against the page's site.
fn guard(key: Key, view: &ICoreWebView2, token: &mut i64) -> Result<()> {
    unsafe {
        // Without source kinds, requests made by frames inside the page would not be seen.
        let sourced = view.cast::<ICoreWebView2_22>().ok();
        // Our own pages come through here too. With Shield off, a page made
        // now hands over nothing else.
        let mut filters = vec![format!("{}://*", crate::omnibox::INTERNAL_SCHEME)];
        if crate::internal::prefs(|p| p.shield) {
            filters.extend(searchwin_shield::filters());
        }
        for filter in filters {
            let filter = HSTRING::from(filter);
            match &sourced {
                Some(view) => view.AddWebResourceRequestedFilterWithRequestSourceKinds(
                    &filter,
                    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                    COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
                )?,
                None => view.AddWebResourceRequestedFilter(&filter, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?,
            }
        }
        view.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut uri = PWSTR::null();
                args.Request()?.Uri(&mut uri)?;
                let uri = take(uri);
                if crate::internal::is_internal(&uri) {
                    if let Some(environment) = with(|p| p.environment.clone()) {
                        serve(&environment, &args, &uri)?;
                    }
                    return Ok(());
                }
                let (page_url, heading_to, environment) = with(|p| {
                    let slot = p.views.get(&key.get());
                    (
                        slot.and_then(|v| v.url.clone()),
                        slot.and_then(|v| v.heading_to.clone()),
                        p.environment.clone(),
                    )
                });
                // Typed or clicked to as the page itself, a tracker's own site still opens.
                if heading_to.as_deref() == Some(uri.as_str()) {
                    return Ok(());
                }
                if crate::shield::blocks(&uri, page_url.as_deref()) {
                    crate::shield::count_blocked();
                    if let Some(environment) = environment {
                        let empty = environment.CreateWebResourceResponse(None, 403, w!("Blocked"), w!(""))?;
                        args.SetResponse(&empty)?;
                    }
                }
                Ok(())
            })),
            token,
        )?;
    }
    Ok(())
}

/// One of our own pages, from inside the exe; an address we have no page for is a 404.
fn serve(environment: &ICoreWebView2Environment, args: &ICoreWebView2WebResourceRequestedEventArgs, uri: &str) -> Result<()> {
    unsafe {
        let response = match crate::internal::page(uri) {
            Some(html) => {
                let stream = CreateStreamOnHGlobal(HGLOBAL::default(), true)?;
                stream.Write(html.as_ptr() as _, html.len() as u32, None).ok()?;
                stream.Seek(0, STREAM_SEEK_SET, None)?;
                let headers = HSTRING::from(crate::internal::HEADERS);
                environment.CreateWebResourceResponse(&stream, 200, w!("OK"), &headers)?
            }
            None => environment.CreateWebResourceResponse(None, 404, w!("Not Found"), w!(""))?,
        };
        args.SetResponse(&response)
    }
}

/// Shield's stylesheet for a page on `url`'s host; our own pages get none: there are no ads to hide.
fn shield_script(url: &str) -> String {
    if crate::internal::is_internal(url) {
        return String::new();
    }
    let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    let css = crate::shield::style_for(&host);
    if css.is_empty() {
        return String::new();
    }
    format!(
        "(()=>{{const s=new CSSStyleSheet();s.replaceSync({});document.adoptedStyleSheets=[...document.adoptedStyleSheets,s]}})()",
        serde_json::json!(css)
    )
}

/// What you hid changed: every page gets the new script now, so a reload
/// already starts without (or with) it.
pub fn refresh_veils() {
    let script = crate::veil::start_script();
    let views: Vec<(TabId, ICoreWebView2)> = with(|p| p.views.iter().filter_map(|(id, v)| Some((*id, v.view.clone()?))).collect());
    for (id, view) in views {
        set_start(id, &view, VEILS, script.clone());
    }
}

/// Sets one of a page's document-start scripts (SHIELD, VEILS), replacing the
/// one it had; empty is none. A script set while a navigation is already under
/// way may come too late for it, so what can be known ahead is set ahead.
fn set_start(id: TabId, view: &ICoreWebView2, which: usize, script: String) {
    let old = with(|p| {
        let slot = &mut p.views.get_mut(&id)?.start[which];
        if slot.text == script {
            return None;
        }
        slot.text = script.clone();
        Some(slot.id.take())
    });
    let Some(old) = old else { return };
    unsafe {
        if let Some(old) = old {
            let _ = view.RemoveScriptToExecuteOnDocumentCreated(&HSTRING::from(old));
        }
        if script.is_empty() {
            return;
        }
        let owner = view.clone();
        let _ = view.AddScriptToExecuteOnDocumentCreated(
            &HSTRING::from(script.as_str()),
            &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(move |result, script_id| {
                result?;
                // Replaced again before this one was in: it goes at once.
                let stale = with(|p| match p.views.get_mut(&id).map(|v| &mut v.start[which]) {
                    Some(slot) if slot.text == script => {
                        slot.id = Some(script_id.clone());
                        false
                    }
                    _ => true,
                });
                if stale {
                    owner.RemoveScriptToExecuteOnDocumentCreated(&HSTRING::from(script_id))?;
                }
                Ok(())
            })),
        );
    }
}

/// A small stream (an icon) read to its end; None if it is empty or unreadable.
fn read_all(stream: &windows::Win32::System::Com::IStream) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let mut read = 0u32;
        let result = unsafe { stream.Read(chunk.as_mut_ptr() as _, chunk.len() as u32, Some(&mut read)) };
        if result.is_err() {
            return None;
        }
        bytes.extend_from_slice(&chunk[..read as usize]);
        // Past a megabyte it is not an icon.
        if read == 0 || bytes.len() > 1 << 20 {
            break;
        }
    }
    (!bytes.is_empty() && bytes.len() <= 1 << 20).then_some(bytes)
}

fn take(text: PWSTR) -> String {
    if text.is_null() {
        return String::new();
    }
    let owned = unsafe { text.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(text.0 as _)) };
    owned
}

fn front_view() -> Option<ICoreWebView2> {
    with(|p| p.front.and_then(|id| p.views.get(&id)).and_then(|v| v.view.clone()))
}

fn front_controller() -> Option<ICoreWebView2Controller> {
    with(|p| p.front.and_then(|id| p.views.get(&id)).and_then(|v| v.controller.clone()))
}

/// Puts tab `id` in the card. `url` is where it should be if it has no page yet
/// (a tab coming back from last session, or one that slept).
pub fn show(id: TabId, url: Option<&str>) {
    let (previous, controller, needs_page, bounds) = with(|p| {
        let previous = p.front.replace(id).filter(|old| *old != id);
        // A page beside it in a split stays on screen.
        // Nor does the floating video, which is somewhere else altogether.
        let previous = previous.filter(|old| !p.panes.iter().any(|(t, _)| t == old) && !p.elsewhere.contains_key(old));
        let previous = previous.and_then(|old| p.views.get(&old)).and_then(|v| v.controller.clone());
        let bounds = p.rect_for(id);
        let away = p.elsewhere.contains_key(&id);
        let slot = p.views.entry(id).or_default();
        if slot.controller.is_none() && slot.pending.is_none() {
            slot.pending = url.map(str::to_string);
        }
        (previous, slot.controller.clone().filter(|_| !away), slot.controller.is_none() && slot.pending.is_some(), bounds)
    });
    unsafe {
        if let Some(previous) = previous {
            let _ = previous.SetIsVisible(false);
        }
        if let Some(controller) = controller {
            let _ = controller.SetBounds(bounds);
            let _ = controller.SetIsVisible(true);
        }
    }
    if needs_page {
        attach(id);
    }
}

/// A page beside the one in front (Split View): made if it has none yet,
/// without taking the front or the keyboard. `arrange` says where it goes.
pub fn wake(id: TabId, url: &str) {
    let needs_page = with(|p| {
        let slot = p.views.entry(id).or_default();
        if slot.controller.is_none() && slot.pending.is_none() {
            slot.pending = Some(url.to_string());
        }
        slot.controller.is_none()
    });
    if needs_page {
        attach(id);
    }
}

/// Where the pages go: `card` for the one in front standing alone, or each
/// pane of the split in front in its own place. Pages that were beside it
/// and no longer are, are hidden.
pub fn arrange(card: RECT, panes: Vec<(TabId, RECT)>) {
    let moves: Vec<(ICoreWebView2Controller, Option<RECT>)> = with(|p| {
        let old = std::mem::replace(&mut p.panes, panes);
        p.bounds = card;
        let mut moves = Vec::new();
        for (id, view) in &p.views {
            let Some(controller) = view.controller.clone() else { continue };
            if p.elsewhere.contains_key(id) {
                continue;
            } else if p.on_screen(*id) {
                moves.push((controller, Some(p.rect_for(*id))));
            } else if old.iter().any(|(t, _)| t == id) {
                moves.push((controller, None));
            }
        }
        moves
    });
    for (controller, place) in moves {
        unsafe {
            match place {
                Some(bounds) => {
                    let _ = controller.SetBounds(bounds);
                    let _ = controller.SetIsVisible(true);
                }
                None => {
                    let _ = controller.SetIsVisible(false);
                }
            }
        }
    }
}

/// Whether pages are side by side now.
pub fn has_panes() -> bool {
    with(|p| !p.panes.is_empty())
}

/// The card and the pages side by side in it, as last arranged.
pub fn placed() -> (RECT, Vec<(TabId, RECT)>) {
    with(|p| (p.bounds, p.panes.clone()))
}

/// A whole row of tabs was put away: pages on their way for it are closed as they come.
pub fn new_epoch() {
    with(|p| {
        p.epoch += 1;
        p.panes.clear();
    });
}

/// Clears away WebView2 profiles (the cookies, sign-ins and site data of a
/// deleted space or profile). A profile is reached through a page made in
/// it, so a hidden one is made and the profile deleted under it
/// (ICoreWebView2Profile8::Delete, which removes the folder once the
/// engine lets it go). Each one gone is reported to crate::spaces; any
/// that isn't is tried again at the next launch.
pub fn erase(names: Vec<String>) {
    let Some((environment, window)) = with(|p| p.environment.clone().map(|e| (e, p.window))) else { return };
    let Ok(environment) = environment.cast::<ICoreWebView2Environment10>() else { return };
    for name in names {
        let done = name.clone();
        let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |result, controller| {
            result?;
            let Some(controller) = controller else { return Ok(()) };
            let deleted = unsafe {
                controller
                    .CoreWebView2()
                    .and_then(|v| v.cast::<ICoreWebView2_13>())
                    .and_then(|v| v.Profile())
                    .and_then(|p| p.cast::<ICoreWebView2Profile8>())
                    .and_then(|p| p.Delete())
            };
            unsafe {
                let _ = controller.Close();
            }
            if deleted.is_ok() {
                crate::spaces::erased(&done);
            }
            Ok(())
        }));
        unsafe {
            let made = environment.CreateCoreWebView2ControllerOptions().and_then(|options| {
                options.SetProfileName(&HSTRING::from(name.as_str()))?;
                environment.CreateCoreWebView2ControllerWithOptions(window, &options, &handler)
            });
            if let Err(error) = made {
                crate::crash_log(&format!("profile {name} not erased: {error}"));
            }
        }
    }
}

pub fn navigate(id: TabId, url: &str) {
    let view = with(|p| {
        let slot = p.views.entry(id).or_default();
        if slot.view.is_none() {
            slot.pending = Some(url.to_string());
        }
        slot.view.clone()
    });
    match view {
        Some(view) => unsafe {
            let _ = view.Navigate(&HSTRING::from(url));
        },
        None => attach(id),
    }
}

/// Lets a tab's page go: it sleeps, or it closed. Its renderer process ends with it.
/// A page out in a window of its own (the floating video) takes that window with it.
pub fn discard(id: TabId) {
    let (controller, away) = with(|p| {
        if p.front == Some(id) {
            p.front = None;
        }
        (p.views.remove(&id).and_then(|v| v.controller), p.elsewhere.remove(&id).is_some())
    });
    if let Some(controller) = controller {
        unsafe {
            let _ = controller.Close();
        }
    }
    if away {
        crate::float::page_gone(id);
    }
    crate::forms::forget(id);
}

/// Puts tab `id`'s page in another window, `parent`, at `rect` in it: the
/// floating video's. It is the same page, so nothing reloads and a video
/// keeps playing; the card no longer moves or hides it. False with no page.
pub fn lift(id: TabId, parent: HWND, rect: RECT) -> bool {
    let Some(controller) = with(|p| {
        let controller = p.views.get(&id)?.controller.clone()?;
        p.elsewhere.insert(id, rect);
        Some(controller)
    }) else {
        return false;
    };
    // Black behind a video, never the card's colour, while it settles in.
    set_background_of(&controller, (0, 0, 0));
    unsafe {
        let _ = controller.SetParentWindow(parent);
        let _ = controller.SetBounds(rect);
        let _ = controller.SetIsVisible(true);
    }
    true
}

/// Back in the window from wherever it was: in the card if its tab is on
/// screen, hidden behind the tab in front otherwise.
pub fn bring_home(id: TabId) {
    let Some((controller, window, rect, shown, background)) = with(|p| {
        p.elsewhere.remove(&id);
        let controller = p.views.get(&id)?.controller.clone()?;
        Some((controller, p.window, p.rect_for(id), p.on_screen(id), p.background))
    }) else {
        return;
    };
    unsafe {
        let _ = controller.SetParentWindow(window);
        let _ = controller.SetBounds(rect);
        let _ = controller.SetIsVisible(shown);
    }
    set_background_of(&controller, background);
}

/// A page out of the card, moved within its own window (resized with it).
pub fn place_elsewhere(id: TabId, rect: RECT) {
    let controller = with(|p| {
        let place = p.elsewhere.get_mut(&id)?;
        *place = rect;
        p.views.get(&id)?.controller.clone()
    });
    if let Some(controller) = controller {
        unsafe {
            let _ = controller.SetBounds(rect);
        }
    }
}

/// The window holding a page out of the card moved on screen.
pub fn moved_elsewhere(id: TabId) {
    if let Some(controller) = with(|p| p.views.get(&id).and_then(|v| v.controller.clone())) {
        unsafe {
            let _ = controller.NotifyParentWindowPositionChanged();
        }
    }
}

/// A peek: a new page at `url`, in `parent` at `rect`, made in the same
/// store as tab `from`'s (a private tab's peek shares its InPrivate
/// profile). Not a tab: its events are heard by nobody until it is kept
/// (rename). The number it goes by, to close or keep it with.
pub fn peek(from: TabId, url: &str, parent: HWND, rect: RECT) -> Option<TabId> {
    let (environment, id, epoch) = with(|p| {
        let environment = p.environment.clone()?;
        p.peeks += 1;
        let id = PEEK_BASE - p.peeks;
        let slot = p.views.entry(id).or_default();
        slot.pending = Some(url.to_string());
        slot.attaching = true;
        p.elsewhere.insert(id, rect);
        Some((environment, id, p.epoch))
    })?;
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |result, controller| {
        result?;
        configure(id, epoch, controller.ok_or_else(|| windows_core::Error::from(E_POINTER))?)
    }));
    let made = match (crate::private_profile(from), crate::spaces::store()) {
        (None, None) => unsafe { environment.CreateCoreWebView2Controller(parent, &handler) },
        (private, store) => unsafe {
            environment.cast::<ICoreWebView2Environment10>().and_then(|environment| {
                let options = environment.CreateCoreWebView2ControllerOptions()?;
                let in_private = private.is_some();
                options.SetProfileName(&HSTRING::from(private.or(store).unwrap_or_default()))?;
                options.SetIsInPrivateModeEnabled(in_private)?;
                environment.CreateCoreWebView2ControllerWithOptions(parent, &options, &handler)
            })
        },
    };
    if let Err(error) = made {
        with(|p| {
            p.views.remove(&id);
            p.elsewhere.remove(&id);
        });
        crate::crash_log(&format!("peek did not open: {error}"));
        return None;
    }
    Some(id)
}

/// A peek kept: its page becomes tab `to`'s, as it is, and its events are
/// that tab's from now on. Still where it was; bring_home puts it in the window.
pub fn rename(from: TabId, to: TabId) {
    with(|p| {
        let Some(view) = p.views.remove(&from) else { return };
        if let Some(key) = &view.key {
            key.set(to);
        }
        if let Some(rect) = p.elsewhere.remove(&from) {
            p.elsewhere.insert(to, rect);
        }
        p.views.insert(to, view);
    });
}

/// Where a page is and what it is called, as it says now.
pub fn address_and_title(id: TabId) -> Option<(String, String)> {
    let view = with(|p| p.views.get(&id).and_then(|v| v.view.clone()))?;
    let (mut source, mut title) = (PWSTR::null(), PWSTR::null());
    unsafe {
        view.Source(&mut source).ok()?;
        let source = take(source);
        let _ = view.DocumentTitle(&mut title);
        Some((source, take(title)))
    }
}

/// Peek turned on or off, or its click changed: every page's next document
/// listens, or doesn't.
pub fn refresh_peek() {
    let script = crate::peek::start_script();
    let views: Vec<(TabId, ICoreWebView2)> = with(|p| p.views.iter().filter_map(|(id, v)| Some((*id, v.view.clone()?))).collect());
    for (id, view) in views {
        set_start(id, &view, PEEK, script.clone());
    }
}

/// Runs a script in tab `id`'s page (or a peek's) without marking anything.
pub fn run(id: TabId, js: &str, done: impl FnOnce(std::result::Result<String, String>) + 'static) {
    match with(|p| p.views.get(&id).and_then(|v| v.view.clone())) {
        Some(view) => execute(&view, js, done),
        None => done(Err("no page".into())),
    }
}

/// The page in front, as the card has it.
pub fn front() -> Option<TabId> {
    with(|p| p.front)
}

/// Runs a script in the front page; `done` gets its result as JSON, or the error.
pub fn eval(js: &str, done: impl FnOnce(std::result::Result<String, String>) + 'static) {
    match front_view() {
        Some(view) => execute(&view, js, done),
        None => done(Err("no page".into())),
    }
}

/// Runs one of our scripts in tab `id`'s page, if it has one, marking the
/// document as given `scripts` (veil's *_IN) first, so what it posts back
/// from the moment it runs is listened to. `done` gets the result as JSON.
pub fn inject(id: TabId, scripts: u8, js: &str, done: impl FnOnce(std::result::Result<String, String>) + 'static) {
    let view = with(|p| {
        let slot = p.views.get_mut(&id)?;
        slot.injected |= scripts;
        slot.view.clone()
    });
    match view {
        Some(view) => execute(&view, js, done),
        None => done(Err("no page".into())),
    }
}

/// Which of our scripts tab `id`'s document was given.
pub fn injected(id: TabId) -> u8 {
    with(|p| p.views.get(&id).map_or(0, |v| v.injected))
}

/// Whether the picker is pointing in tab `id`.
pub fn picking(id: TabId) -> bool {
    with(|p| p.views.get(&id).is_some_and(|v| v.picking))
}

pub fn set_picking(id: TabId, on: bool) {
    with(|p| {
        if let Some(slot) = p.views.get_mut(&id) {
            slot.picking = on;
        }
    });
}

fn execute(view: &ICoreWebView2, js: &str, done: impl FnOnce(std::result::Result<String, String>) + 'static) {
    let done = std::cell::Cell::new(Some(done));
    let handler = ExecuteScriptCompletedHandler::create(Box::new(move |result, json| {
        if let Some(done) = done.take() {
            done(result.map(|_| json).map_err(|e| e.to_string()));
        }
        Ok(())
    }));
    unsafe {
        let _ = view.ExecuteScript(&HSTRING::from(js), &handler);
    }
}

/// News for every open page of ours at search://`host` (the downloads' progress,
/// a bookmark to edit). Each page is asked where it is right then: one that went
/// on to a website gets nothing.
pub fn post_to_internal(host: &str, json: &str) {
    let views: Vec<ICoreWebView2> = with(|p| {
        p.views
            .values()
            .filter(|v| v.url.as_deref().and_then(|u| url::Url::parse(u).ok()).is_some_and(|u| {
                u.scheme() == crate::omnibox::INTERNAL_SCHEME && u.host_str() == Some(host)
            }))
            .filter_map(|v| v.view.clone())
            .collect()
    });
    for view in views {
        let mut now = PWSTR::null();
        let here = unsafe { view.Source(&mut now) }.is_ok() && crate::internal::is_internal(&take(now));
        if here {
            unsafe {
                let _ = view.PostWebMessageAsJson(&HSTRING::from(json));
            }
        }
    }
}

/// News for tab `id`'s page alone, only while it is still our search://`host`
/// (a password shown to the one Passwords page that asked for it).
pub fn post_to(id: TabId, host: &str, json: &str) -> bool {
    let Some(view) = with(|p| p.views.get(&id).and_then(|v| v.view.clone())) else { return false };
    let mut now = PWSTR::null();
    let here = unsafe { view.Source(&mut now) }.is_ok()
        && url::Url::parse(&take(now)).is_ok_and(|u| u.scheme() == crate::omnibox::INTERNAL_SCHEME && u.host_str() == Some(host));
    here && unsafe { view.PostWebMessageAsJson(&HSTRING::from(json)) }.is_ok()
}

/// A box in tab `id`'s page, given in the page's CSS pixels from its top
/// left, on the screen: the page's zoom and the window's DPI applied, cut to
/// the page's own place; with that place on the screen. None for a page that
/// isn't in the card (a peek's, the floating video's), or a box outside it.
pub fn to_screen(id: TabId, x: f64, y: f64, w: f64, h: f64) -> Option<(RECT, RECT)> {
    let (controller, place, window) = with(|p| {
        let controller = p.views.get(&id)?.controller.clone()?;
        (!p.elsewhere.contains_key(&id) && p.on_screen(id)).then(|| (controller, p.rect_for(id), p.window))
    })?;
    let mut zoom = 1.0;
    unsafe {
        let _ = controller.ZoomFactor(&mut zoom);
    }
    let scale = zoom * f64::from(crate::dpi()) / 96.0;
    let px = |v: f64| (v * scale).round() as i32;
    let (left, top) = (place.left + px(x), place.top + px(y));
    let inside = clip(RECT { left, top, right: left + px(w), bottom: top + px(h) }, place)?;
    let mut origin = windows::Win32::Foundation::POINT::default();
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::ClientToScreen(window, &mut origin);
    }
    let on_screen = |r: RECT| RECT { left: r.left + origin.x, top: r.top + origin.y, right: r.right + origin.x, bottom: r.bottom + origin.y };
    Some((on_screen(inside), on_screen(place)))
}

/// `b` cut to `room`; None when nothing of it is inside.
fn clip(b: RECT, room: RECT) -> Option<RECT> {
    let cut = RECT { left: b.left.max(room.left), top: b.top.max(room.top), right: b.right.min(room.right), bottom: b.bottom.min(room.bottom) };
    (cut.left < cut.right && cut.top < cut.bottom).then_some(cut)
}

/// Runs a script in a frame of a page (crate::forms fills a sign-in there).
pub fn run_in_frame(frame: &ICoreWebView2Frame2, js: &str, done: impl FnOnce(std::result::Result<String, String>) + 'static) {
    let done = std::cell::Cell::new(Some(done));
    let handler = ExecuteScriptCompletedHandler::create(Box::new(move |result, json| {
        if let Some(done) = done.take() {
            done(result.map(|_| json).map_err(|e| e.to_string()));
        }
        Ok(())
    }));
    unsafe {
        let _ = frame.ExecuteScript(&HSTRING::from(js), &handler);
    }
}

pub fn has_page(id: TabId) -> bool {
    with(|p| p.views.get(&id).is_some_and(|v| v.view.is_some()))
}

/// Whether the page in front can go back, and forward.
pub fn can_go() -> (bool, bool) {
    let Some(view) = front_view() else { return (false, false) };
    let (mut back, mut forward) = (BOOL::default(), BOOL::default());
    unsafe {
        let _ = view.CanGoBack(&mut back);
        let _ = view.CanGoForward(&mut forward);
    }
    (back.as_bool(), forward.as_bool())
}

pub fn reload() {
    if let Some(view) = front_view() {
        unsafe {
            let _ = view.Reload();
        }
    }
}

pub fn back() {
    if let Some(view) = front_view() {
        let mut can = BOOL::default();
        unsafe {
            if view.CanGoBack(&mut can).is_ok() && can.as_bool() {
                let _ = view.GoBack();
            }
        }
    }
}

pub fn forward() {
    if let Some(view) = front_view() {
        let mut can = BOOL::default();
        unsafe {
            if view.CanGoForward(&mut can).is_ok() && can.as_bool() {
                let _ = view.GoForward();
            }
        }
    }
}

pub fn focus() {
    if let Some(controller) = front_controller() {
        unsafe {
            let _ = controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        }
    }
}

/// Minimized, the pages stop drawing and give their timers a rest.
pub fn set_visible(visible: bool) {
    for controller in with(|p| p.shown_controllers()) {
        unsafe {
            let _ = controller.SetIsVisible(visible);
        }
    }
}

pub fn window_moved() {
    for controller in with(|p| p.shown_controllers()) {
        unsafe {
            let _ = controller.NotifyParentWindowPositionChanged();
        }
    }
}

/// What shows before a page has drawn: the card's colour, never a white flash in dark mode.
pub fn set_background(color: (u8, u8, u8)) {
    let controllers: Vec<_> = with(|p| {
        p.background = color;
        p.views.iter().filter(|(id, _)| !p.elsewhere.contains_key(id)).filter_map(|(_, v)| v.controller.clone()).collect()
    });
    for controller in controllers {
        set_background_of(&controller, color);
    }
}

fn set_background_of(controller: &ICoreWebView2Controller, (r, g, b): (u8, u8, u8)) {
    if let Ok(controller) = controller.cast::<ICoreWebView2Controller2>() {
        unsafe {
            let _ = controller.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR { A: 255, R: r, G: g, B: b });
        }
    }
}

pub fn close_all() {
    let controllers: Vec<_> = with(|p| p.views.drain().filter_map(|(_, v)| v.controller).collect());
    for controller in controllers {
        unsafe {
            let _ = controller.Close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_is_cut_to_the_page() {
        let room = RECT { left: 0, top: 100, right: 800, bottom: 600 };
        let r = |left, top, right, bottom| RECT { left, top, right, bottom };
        assert_eq!(clip(r(10, 200, 110, 230), room), Some(r(10, 200, 110, 230)));
        assert_eq!(clip(r(-50, 580, 50, 620), room), Some(r(0, 580, 50, 600)));
        // Over the toolbar, or off the page: nothing to hang a list from.
        assert_eq!(clip(r(10, 20, 110, 90), room), None);
        assert_eq!(clip(r(900, 200, 1000, 230), room), None);
    }
}
