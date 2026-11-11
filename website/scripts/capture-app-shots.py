# Captures the app screens used by the product-tour video (demo/shots/*.png).
#
# These are the real Windows UI (apps/windows/ui), not mock-ups: the built UI is served on
# its own and every Tauri call is answered from demo data in demo/capture/, the same idea
# as apps/windows/ui/e2e/fakeBridge.ts. Times written "@now<minutes>" become real times.
#
#   cd apps/windows/ui && npx npm@11 ci && npx vite build
#   python3 -m http.server 8792 --directory apps/windows/ui/dist &
#   python3 website/scripts/capture-app-shots.py                         # today, meeting, topic, ask
#   FX=fixtures-live.json python3 website/scripts/capture-app-shots.py   # live (recording)
#
# Needs Python Playwright and Google Chrome. Sidebar icons are hidden because they are
# drawn with Segoe Fluent Icons, a Windows font; on Windows they show. Settings is not
# captured: it needs more demo data than demo/capture/ has.
import sys, json, pathlib, time
from playwright.sync_api import sync_playwright
S = pathlib.Path(__file__).parent.parent / "demo" / "capture"
OUT = pathlib.Path(__file__).parent.parent / "demo" / "shots"
import os
fx = json.loads((S/os.environ.get("FX","fixtures.json")).read_text())
ALL = [{"shot":"today"},{"tab":"Meetings"},{"wait":1500},{"shot":"meeting"},{"tab":"Topics"},{"wait":1500},{"shot":"topic"},
       {"tab":"Library"},{"wait":900},{"click":"Ask"},{"wait":1200},{"click":"Acme pricing and what I owe"},{"wait":1500},{"shot":"ask"}]
LIVE = [{"tab":"Meetings"},{"wait":1500},{"shot":"live"}]
steps = json.loads(sys.argv[1]) if len(sys.argv)>1 else (LIVE if os.environ.get("FX","").startswith("fixtures-live") else ALL)
INIT = """
(() => {
  const data = __DATA__;
  const now = Date.now();
  const fix = (v) => JSON.parse(JSON.stringify(v), (k, x) => (typeof x === 'string' && x.startsWith('@now')) ? now + Number(x.slice(4) || 0) * 60000 : x);
  window.__calls = [];
  try { localStorage.setItem('appearance','light'); } catch {}
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } },
    transformCallback: () => 0,
    invoke: async (cmd, args) => { window.__calls.push(cmd + (cmd in data ? '' : ' !MISSING ' + JSON.stringify(args||{}).slice(0,120))); return cmd in data ? fix(data[cmd]) : null; },
    convertFileSrc: (s) => s,
  };
})();
"""
with sync_playwright() as p:
    b = p.chromium.launch(channel="chrome")
    c = b.new_context(viewport={"width":1040,"height":600}, device_scale_factor=2)
    pg = c.new_page()
    errs=[]
    pg.on("pageerror", lambda e: errs.append(str(e)[:300]))
    pg.add_init_script(INIT.replace("__DATA__", json.dumps(fx)))
    pg.goto("http://localhost:8792/"); pg.wait_for_timeout(1500)
    pg.add_style_tag(content=".skin-icon{display:none!important}")
    for st in steps:
        if "tab" in st:
            try: pg.get_by_role("tab", name=st["tab"]).first.click(timeout=4000)
            except Exception as e: print("TABFAIL", st["tab"])
        if "click" in st:
            try: pg.get_by_text(st["click"], exact=st.get("exact", True)).first.click(timeout=4000)
            except Exception as e: print("CLICKFAIL", st["click"], str(e)[:120])
        if "fill" in st:
            pg.locator(st["sel"]).first.fill(st["fill"])
        if "press" in st: pg.keyboard.press(st["press"])
        if "wait" in st: pg.wait_for_timeout(st["wait"])
        if "shot" in st:
            pg.wait_for_timeout(600); pg.screenshot(path=str(OUT/f"{st['shot']}.png"))
    calls = pg.evaluate("[...new Set(window.__calls)]")
    print("\n".join(sorted(calls))); print("ERRS", errs[:5])
    b.close()
