#!/usr/bin/env bash
# update_satya_nocache.sh - stop the desktop window from showing a stale UI.
# Adds "Cache-Control: no-store" to every response from the SATYA server.
# Usage: bash update_satya_nocache.sh /path/to/satya     (does not build)
set -euo pipefail
cd "${1:-.}" || exit 1
f=crates/satya-api/src/lib.rs
[ -f "$f" ] || { echo "ERROR: run from the SATYA repo root" >&2; exit 1; }
if grep -q "async fn no_cache" "$f"; then echo "Already applied."; exit 0; fi
TMP="$(mktemp)"; trap 'rm -f "$TMP"' EXIT
cat > "$TMP" <<'__SATYA_EOF__'
--- a/crates/satya-api/src/lib.rs
+++ b/crates/satya-api/src/lib.rs
@@ -116,9 +116,17 @@
         .route("/api/settings", get(settings_api::get).post(settings_api::put))
         .route("/api/settings/test", post(settings_api::test))
         .fallback_service(web)
+        // The desktop window (WebKit) caches aggressively; without this it can
+        // keep showing an old UI after an update.
+        .layer(axum::middleware::map_response(no_cache))
         .with_state(state)
 }

+async fn no_cache(mut res: Response) -> Response {
+    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
+    res
+}
+
 // ---------------------------------------------------------------------------
 // Case
 // ---------------------------------------------------------------------------
__SATYA_EOF__
if command -v git >/dev/null 2>&1; then CHECK=(git apply --check "$TMP"); APPLY=(git apply "$TMP")
else CHECK=(patch -p1 --dry-run -s -i "$TMP"); APPLY=(patch -p1 -s -i "$TMP"); fi
"${CHECK[@]}" || { echo "ERROR: patch does not apply; nothing changed" >&2; exit 1; }
mkdir -p .satya_backup_nocache && cp -p "$f" .satya_backup_nocache/lib.rs
"${APPLY[@]}"
echo "Done: $f now sends Cache-Control: no-store (backup in .satya_backup_nocache/)."
echo "Rebuild, then clear the old cache once:"
echo "  rm -rf ~/.cache/org.satya.forensics ~/.local/share/org.satya.forensics"
