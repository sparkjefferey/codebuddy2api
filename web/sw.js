/* API Transmitter · Service Worker
 * 策略：
 *   - 静态资源(壳): 缓存优先 -> 网络回填, 安装时预缓存应用壳
 *   - API 请求(/health /v1/* /agents /settings /credits): 网络优先, 绝不缓存;
 *     网关注掉线时返回一个离线提示 JSON。  */
const CACHE = "wb-shell-v1";
const SHELL = ["/", "/manifest.webmanifest", "/icons/icon-192.png", "/icons/icon-512.png"];

const isAPI = (u) =>
  /^\/(health|v1\/|agents|settings|credits|models\/reload|ccswitch)/.test(new URL(u).pathname);

self.addEventListener("install", (e) => {
  e.waitUntil(
    caches.open(CACHE).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting())
  );
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches.keys().then((ks) =>
      Promise.all(ks.filter((k) => k !== CACHE).map((k) => caches.delete(k)))
    ).then(() => self.clients.claim())
  );
});

self.addEventListener("fetch", (e) => {
  const req = e.request;
  const url = new URL(req.url);

  // 跨源或非 GET: 一律网络, 不缓存
  if (url.origin !== self.location.origin || req.method !== "GET") return;

  // API: 网络优先, 失败给离线占位(不落缓存)
  if (isAPI(url.pathname)) {
    e.respondWith(
      fetch(req).catch(() =>
        new Response(JSON.stringify({
          error: { message: "网关离线:请先在目标机器启动 WorkBuddy 网关服务", type: "offline" }
        }), { status: 503, headers: { "Content-Type": "application/json" } })
      )
    );
    return;
  }

  // 静态壳: cache-first -> 网络回填并缓存
  e.respondWith(
    caches.match(req).then((hit) => {
      if (hit) return hit;
      return fetch(req).then((res) => {
        const copy = res.clone();
        if (res.ok) caches.open(CACHE).then((c) => c.put(req, copy));
        return res;
      }).catch(() => caches.match("/"));
    })
  );
});