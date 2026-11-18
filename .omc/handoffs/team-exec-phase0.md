## Handoff: team-exec (Phase 0) → team-exec (Phase 1)

- **Decided**: user confirmed start with defaults; priority is speed, lightness, minimal UI. Unpackaged, self-contained WinUI 3 (WindowsAppSDK.WinUI + .Foundation 2.6.5 only), WebView2 shared environment warmed at launch, lazy per-page attach. ReadyToRun + trim publish.
- **Rejected**: WindowsAppSDK metapackage (+60 MB AI/ML); NativeAOT for now (needs VS C++ Build Tools — pending user decision).
- **Risks**: trimming WinUI without AOT kept TrimmerRootAssembly for Microsoft.WinUI and WebView2 projection (harmless; revisit when AOT lands). Keyboard accelerators while WebView2 has focus not yet verified. Card corner clipping over WebView2 not effective.
- **Files**: SearchWin.slnx, src/SearchWin.Core/{Address,Engine,Omnibox}.cs, src/SearchWin.App/{App,MainWindow,PageEngine,CrashLog}*, tests/SearchWin.Core.Tests/AddressTests.cs, PLANO.md (Phase 1 task table).
- **Remaining**: Phase 1 tasks #1–#7 in PLANO.md; baseline to beat: first page ~640 ms warm, app 80 MB private.
