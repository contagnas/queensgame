pub fn render_app_page(title: &str, description: &str, app_json: &str) -> String {
    let app_json = app_json.replace('<', "\\u003c");

    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{title}</title><meta name="description" content="{description}"><link rel="icon" type="image/svg+xml" href="/static/mage-light.svg"><script>(function(){{try{{var key="boardmage:theme";var stored=localStorage.getItem(key);var dark=stored?stored==="dark":window.matchMedia&&window.matchMedia("(prefers-color-scheme: dark)").matches;document.documentElement.dataset.theme=dark?"dark":"light";}}catch(_){{document.documentElement.dataset.theme="light";}}}})();</script><link rel="stylesheet" href="/static/style.css"></head><body><div id="game-root"></div><script type="application/json" id="app-data">{app_json}</script><script type="module">import init from "/static/client/queensgame_client.js"; init();</script></body></html>"#
    )
}
