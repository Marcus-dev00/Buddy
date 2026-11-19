/**
 * Buddy Companion Cloudflare Worker:
 * - Serves Official Website (`/`) & Online BuddyMobile PWA (`/app/`) & APK (`/downloads/BuddyMobile.apk`) via Workers Static Assets.
 * - Provides zero-knowledge Dynamic Tunnel Directory (`/api/directory/*`) backed by Cloudflare KV (`BUDDY_DIR`)
 *   so paired phones automatically find the PC's latest free Cloudflare Quick Tunnel URL after reboots.
 */

const CORS_HEADERS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type",
};

function jsonResponse(data, status = 200) {
  return new Response(JSON.stringify(data), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      "Cache-Control": "no-store",
      ...CORS_HEADERS,
    },
  });
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const path = url.pathname;

    if (request.method === "OPTIONS" && path.startsWith("/api/")) {
      return new Response(null, { status: 204, headers: CORS_HEADERS });
    }

    // Dynamic Tunnel Directory: Update (called by Desktop Buddy)
    if (request.method === "POST" && path === "/api/directory/update") {
      try {
        const body = await request.json();
        const pairId = String(body.pair_id || "").trim();
        const authHash = String(body.auth_hash || "").trim();
        const remoteUrl = String(body.remote_url || "").trim();
        const lanUrl = String(body.lan_url || "").trim();

        if (!/^[a-f0-9]{12,64}$/i.test(pairId) || !/^[a-f0-9]{64}$/i.test(authHash)) {
          return jsonResponse({ ok: false, error: "Invalid pair_id or auth_hash" }, 400);
        }
        if (remoteUrl && !remoteUrl.startsWith("https://") && !remoteUrl.startsWith("http://")) {
          return jsonResponse({ ok: false, error: "Invalid remote_url" }, 400);
        }

        const key = `dir:${pairId}`;
        const existingRaw = await env.BUDDY_DIR.get(key);
        if (existingRaw) {
          const existing = JSON.parse(existingRaw);
          if (existing.auth_hash && existing.auth_hash !== authHash) {
            return jsonResponse({ ok: false, error: "Unauthorized" }, 403);
          }
        }

        await env.BUDDY_DIR.put(
          key,
          JSON.stringify({
            auth_hash: authHash,
            remote_url: remoteUrl,
            lan_url: lanUrl,
            updated_at: Date.now(),
          }),
          { expirationTtl: 14 * 24 * 3600 }
        );

        return jsonResponse({ ok: true });
      } catch (e) {
        return jsonResponse({ ok: false, error: String(e) }, 400);
      }
    }

    // Dynamic Tunnel Directory: Resolve (called by BuddyMobile App when LAN/last remote unreachable)
    if (request.method === "GET" && path === "/api/directory/resolve") {
      const pairId = (url.searchParams.get("pair_id") || "").trim();
      const authHash = (url.searchParams.get("auth_hash") || "").trim();
      if (!/^[a-f0-9]{12,64}$/i.test(pairId) || !/^[a-f0-9]{64}$/i.test(authHash)) {
        return jsonResponse({ ok: false, error: "Invalid query" }, 400);
      }
      const raw = await env.BUDDY_DIR.get(`dir:${pairId}`);
      if (!raw) {
        return jsonResponse({ ok: false, error: "Not found" }, 404);
      }
      const record = JSON.parse(raw);
      if (record.auth_hash !== authHash) {
        return jsonResponse({ ok: false, error: "Unauthorized" }, 403);
      }
      return jsonResponse({
        ok: true,
        remote_url: record.remote_url || "",
        lan_url: record.lan_url || "",
        updated_at: record.updated_at || 0,
      });
    }

    // Clean redirect `/app` -> `/app/` so relative `./assets/...` paths always resolve cleanly
    if (path === "/app") {
      return Response.redirect(`${url.origin}/app/`, 301);
    }

    if (env.ASSETS) {
      return env.ASSETS.fetch(request);
    }
    return new Response("Not Found", { status: 404 });
  },
};
