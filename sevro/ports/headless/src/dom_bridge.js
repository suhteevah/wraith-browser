// Wraith DOM Bridge — injected into QuickJS before page scripts run.
// Provides document.*, window.*, navigator.*, setTimeout, fetch stubs.

var __wraith_nodes = {node_json};
var __wraith_node_index = {};
for (var i = 0; i < __wraith_nodes.length; i++) {
    var n = __wraith_nodes[i];
    if (n.id) __wraith_node_index[n.id] = n;
}

// === document ===
if (typeof document === 'undefined') var document = {};

document.querySelector = function(sel) {
    for (var i = 0; i < __wraith_nodes.length; i++) {
        var n = __wraith_nodes[i];
        if (sel.startsWith('#') && n.id === sel.substring(1)) return n;
        if (sel.startsWith('.') && n.className && n.className.indexOf(sel.substring(1)) >= 0) return n;
        if (sel === n.tag) return n;
        var m = sel.match(/^(\w+)\[(\w+)=["']?([^"'\]]+)["']?\]$/);
        if (m && n.tag === m[1] && n.attrs && n.attrs[m[2]] === m[3]) return n;
    }
    return null;
};

document.querySelectorAll = function(sel) {
    var results = [];
    for (var i = 0; i < __wraith_nodes.length; i++) {
        var n = __wraith_nodes[i];
        if (sel.startsWith('#') && n.id === sel.substring(1)) results.push(n);
        else if (sel.startsWith('.') && n.className && n.className.indexOf(sel.substring(1)) >= 0) results.push(n);
        else if (sel === n.tag) results.push(n);
    }
    return results;
};

document.getElementById = function(id) {
    return __wraith_node_index[id] || null;
};

document.getElementsByTagName = function(tag) {
    return __wraith_nodes.filter(function(n) { return n.tag === tag; });
};

document.getElementsByClassName = function(cls) {
    return __wraith_nodes.filter(function(n) {
        return n.className && n.className.indexOf(cls) >= 0;
    });
};

document.createElement = function(tag) {
    return { tag: tag, attrs: {}, textContent: '', children: [], className: '',
             setAttribute: function(k,v) { this.attrs[k] = v; },
             getAttribute: function(k) { return this.attrs[k]; },
             appendChild: function(c) { this.children.push(c); },
             addEventListener: function() {} };
};

document.createTextNode = function(text) {
    return { textContent: text, nodeType: 3 };
};

document.createDocumentFragment = function() {
    return { children: [], appendChild: function(c) { this.children.push(c); } };
};

document.title = "{title}";
document.readyState = "complete";
document.body = document.querySelector('body') || { appendChild: function() {}, innerHTML: '' };
document.head = document.querySelector('head') || { appendChild: function() {} };
document.documentElement = document.querySelector('html') || { lang: 'en' };

// Event listeners (no-op stubs)
document.addEventListener = function() {};
document.removeEventListener = function() {};
document.dispatchEvent = function() { return true; };

// === window ===
if (typeof window === 'undefined') var window = {};
window.document = document;
window.addEventListener = function() {};
window.removeEventListener = function() {};
window.dispatchEvent = function() { return true; };
window.getComputedStyle = function() { return {}; };
window.matchMedia = function(q) { return { matches: false, media: q, addListener: function() {}, removeListener: function() {} }; };
window.requestAnimationFrame = function(cb) { cb(Date.now()); return 1; };
window.cancelAnimationFrame = function() {};
window.innerWidth = 1920;
window.innerHeight = 1080;
window.scrollX = 0;
window.scrollY = 0;
window.scrollTo = function() {};
window.scroll = function() {};
window.pageXOffset = 0;
window.pageYOffset = 0;
window.devicePixelRatio = 1;

// === location ===
document.location = { href: '', hostname: '', pathname: '/', protocol: 'https:', search: '', hash: '' };
window.location = document.location;

// === setTimeout / setInterval ===
// Execute callbacks immediately (delay=0 semantics).
// This is sufficient for most SPA hydration code that uses setTimeout(fn, 0).
var __wraith_timer_id = 0;
var __wraith_pending_timers = [];

window.setTimeout = function(fn, delay) {
    __wraith_timer_id++;
    if (typeof fn === 'function') {
        __wraith_pending_timers.push({ id: __wraith_timer_id, fn: fn });
    }
    return __wraith_timer_id;
};

window.setInterval = function(fn, delay) {
    // Execute once (no real interval in sync mode)
    return window.setTimeout(fn, delay);
};

window.clearTimeout = function(id) {
    __wraith_pending_timers = __wraith_pending_timers.filter(function(t) { return t.id !== id; });
};

window.clearInterval = window.clearTimeout;

// Global aliases
var setTimeout = window.setTimeout;
var setInterval = window.setInterval;
var clearTimeout = window.clearTimeout;
var clearInterval = window.clearInterval;

// Flush pending timers (called after scripts run)
function __wraith_flush_timers() {
    var maxIterations = 100;
    var iteration = 0;
    while (__wraith_pending_timers.length > 0 && iteration < maxIterations) {
        var batch = __wraith_pending_timers.splice(0);
        for (var i = 0; i < batch.length; i++) {
            try { batch[i].fn(); } catch(e) {}
        }
        iteration++;
    }
}

// === fetch stub ===
// Returns a thenable that resolves with a Response-like object.
// For actual HTTP, the Rust side handles networking.
window.fetch = function(url, options) {
    return {
        then: function(resolve, reject) {
            // Stub: return empty response
            if (resolve) {
                resolve({
                    ok: true,
                    status: 200,
                    url: url,
                    json: function() { return { then: function(r) { r({}); } }; },
                    text: function() { return { then: function(r) { r(''); } }; },
                    headers: { get: function() { return null; } }
                });
            }
            return this;
        },
        catch: function() { return this; },
        finally: function(fn) { if (fn) fn(); return this; }
    };
};
var fetch = window.fetch;

// === navigator ===
if (typeof navigator === 'undefined') var navigator = {};
navigator.userAgent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
navigator.language = "en-US";
navigator.languages = ["en-US", "en"];
navigator.platform = "Win32";
navigator.cookieEnabled = true;
navigator.onLine = true;
navigator.hardwareConcurrency = 8;
navigator.maxTouchPoints = 0;
navigator.vendor = "Google Inc.";
navigator.userAgentData = { brands: [], mobile: false, platform: "Windows" };
navigator.mediaDevices = { enumerateDevices: function() { return { then: function(r) { r([]); } }; } };
navigator.permissions = { query: function() { return { then: function(r) { r({ state: 'granted' }); } }; } };
navigator.clipboard = { readText: function() { return { then: function(r) { r(''); } }; } };

// === MutationObserver stub ===
window.MutationObserver = function() {
    this.observe = function() {};
    this.disconnect = function() {};
    this.takeRecords = function() { return []; };
};

// === IntersectionObserver stub ===
window.IntersectionObserver = function(cb) {
    this.observe = function() {};
    this.unobserve = function() {};
    this.disconnect = function() {};
};

// === ResizeObserver stub ===
window.ResizeObserver = function() {
    this.observe = function() {};
    this.unobserve = function() {};
    this.disconnect = function() {};
};

// === CustomEvent ===
window.CustomEvent = function(type, params) {
    this.type = type;
    this.detail = params ? params.detail : null;
};

// === DOMParser stub ===
window.DOMParser = function() {
    this.parseFromString = function(str, type) { return document; };
};

// === localStorage / sessionStorage stubs ===
var __wraith_storage = {};
var __wraith_make_storage = function() {
    return {
        getItem: function(k) { return __wraith_storage[k] || null; },
        setItem: function(k, v) { __wraith_storage[k] = String(v); },
        removeItem: function(k) { delete __wraith_storage[k]; },
        clear: function() { __wraith_storage = {}; },
        get length() { return Object.keys(__wraith_storage).length; }
    };
};
window.localStorage = __wraith_make_storage();
window.sessionStorage = __wraith_make_storage();
var localStorage = window.localStorage;
var sessionStorage = window.sessionStorage;

// === Array.from polyfill ===
if (!Array.from) {
    Array.from = function(obj) {
        return Array.prototype.slice.call(obj);
    };
}

// === performance stub ===
window.performance = {
    now: function() { return Date.now(); },
    mark: function() {},
    measure: function() {},
    getEntriesByType: function() { return []; },
    getEntriesByName: function() { return []; }
};
