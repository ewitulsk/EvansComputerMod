-- auth: accounts on an Auth server (authd) or a backend's own realm (man auth).
-- The same names as Python's auth (auth.py).
--
--   local auth = require("auth")
--
--   local me = auth.signup("10.0.8.1", "alice", "hunter2", "Alice")
--   print(me.realm, auth.display_fingerprint(me.fingerprint))
--   local tok = auth.token(backend_fingerprint)        -- for one backend, 1 hour
--   local proof = auth.prove(nonce_hex, backend_fingerprint)
--
-- The password never leaves this computer: it becomes a key for that realm
-- only, and everything after the handshake is encrypted. The first realm seen
-- at a host is pinned (trusted on first use); a realm whose name or address
-- matches a pin but whose key changed is refused (a Lua error starting with
-- "auth: the identity of ...") until the pin is removed by hand with
-- auth.unpin(fingerprint). Logins and pins live in .auth/ next to the script
-- (or in the current directory). Errors are Lua errors starting with "auth: ".

local native = require("_auth")

local auth = {}

auth.DEFAULT_PORT = native.DEFAULT_PORT

local function dir()
  local a = rawget(_G, "arg")
  local s = type(a) == "table" and a[0]
  if type(s) == "string" then
    local d = s:match("^(.*)/[^/]*%.lua$")
    if d then return d .. "/.auth" end
  end
  return ".auth"
end

-- Sign in to the realm at host ("host" or "host:port", port 7600) with a password.
function auth.signin(host, user, password)
  return native.signin(dir(), host, user, password)
end

-- Make an account (username: 1-16 of a-z 0-9 . _ -) and sign in.
function auth.signup(host, user, password, display)
  if type(display) == "table" then display = display.display end
  return native.signup(dir(), host, user, password, display)
end

-- A token for the backend whose key fingerprint is audience, from the realm at
-- host (or the realm signed in to last).
function auth.token(audience, host)
  return native.token(dir(), audience, host)
end

-- The proof (hex) a backend asks for with its connect nonce (hex).
function auth.prove(nonce, audience, host)
  return native.prove(dir(), nonce, audience, host)
end

-- The fingerprint of the realm at host, without signing in.
function auth.fingerprint(host)
  local fp = native.fingerprint(dir(), host)
  return fp
end

-- Sign out of the realm at host (or the last one). false if not signed in.
function auth.logout(host)
  return native.logout(dir(), host)
end

-- The pinned realms: { {fingerprint=, host=, realm=}, ... }.
function auth.pins()
  return native.pins(dir())
end

-- Remove a pin by hand (a fingerprint or a host), and its saved login.
function auth.unpin(which)
  return native.unpin(dir(), which)
end

-- 3fa19c... -> 3F:A1:9C:...
function auth.display_fingerprint(fp)
  return native.display_fingerprint(fp)
end

return auth
