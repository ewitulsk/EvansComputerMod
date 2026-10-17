"""Accounts on an Auth server (authd) or a backend's own realm (man auth).

    import auth

    me = auth.signup("10.0.8.1", "alice", "hunter2", display="Alice")
    print(me["realm"], auth.display_fingerprint(me["fingerprint"]))
    tok = auth.token(backend_fingerprint)          # for one backend, 1 hour
    proof = auth.prove(nonce_hex, backend_fingerprint)

The password never leaves this computer: it becomes a key for that realm
only, and everything after the handshake is encrypted. The first realm seen
at a host is pinned (trusted on first use); a realm whose name or address
matches a pin but whose key changed is refused with AuthError until the pin
is removed by hand with auth.unpin(fingerprint). Logins and pins live in
.auth/ next to the script (or in the current directory). Lua's auth module
has the same functions.
"""

import _auth

DEFAULT_PORT = _auth.DEFAULT_PORT


class AuthError(Exception):
    """A refused sign-in, a changed realm key, a network problem."""


def _dir():
    import sys
    p = sys.argv[0] if getattr(sys, "argv", None) else ""
    if p.endswith(".py") and "/" in p:
        return p[:p.rfind("/")] + "/.auth"
    return ".auth"


def _call(fn, *args):
    try:
        return fn(_dir(), *args)
    except RuntimeError as e:
        raise AuthError(str(e)) from None


def _login(row):
    return {"realm": row[0], "fingerprint": row[1], "host": row[2], "user": row[3], "display": row[4], "id": row[5]}


def signin(host, user, password):
    """Sign in to the realm at host ("host" or "host:port", port 7600) with a password."""
    return _login(_call(_auth.signin, host, user, password))


def signup(host, user, password, display=None):
    """Make an account (username: 1-16 of a-z 0-9 . _ -) and sign in."""
    return _login(_call(_auth.signup, host, user, password, display))


def token(audience, host=None):
    """A token for the backend whose key fingerprint is audience, from the realm at
    host (or the realm signed in to last)."""
    return _call(_auth.token, audience, host)


def prove(nonce, audience, host=None):
    """The proof (hex) a backend asks for with its connect nonce (hex)."""
    return _call(_auth.prove, nonce, audience, host)


def fingerprint(host):
    """The fingerprint of the realm at host, without signing in."""
    return _call(_auth.fingerprint, host)[0]


def logout(host=None):
    """Sign out of the realm at host (or the last one). False if not signed in."""
    return _call(_auth.logout, host)


def pins():
    """The pinned realms: [{"fingerprint", "host", "realm"}, ...]."""
    return [{"fingerprint": r[0], "host": r[1], "realm": r[2]} for r in _call(_auth.pins)]


def unpin(which):
    """Remove a pin by hand (a fingerprint or a host), and its saved login."""
    return _call(_auth.unpin, which)


def display_fingerprint(fp):
    """3fa19c... -> 3F:A1:9C:..."""
    return _auth.display_fingerprint(fp)
