# Global window geometry; Diff reopen binds Comparison

Main and Diff OS window bounds persist in `window_geometry.json` (app-global, not Workspace). Splitters stay session-only (ADR-0004); Settings is not restored. Diff auto-reopens only if it was open at quit, rebuilt from the persisted Comparison `(repository, base_oid, head_oid)` plus `selected_path`—independent of the restored Workspace (ADR-0005). Invalid/off-screen bounds fall back to centered defaults; maximized is not modeled in v1.
