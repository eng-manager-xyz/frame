# frame-windows-capture-ffi

Audited Win32 boundary for Frame's Windows capture adapter. It exposes only
privacy-filtered, label-free monitor/window geometry, capture-item
construction, and a thread stop signal. Titles, process names, device names,
raw handles, and pointers do not cross the public API.
