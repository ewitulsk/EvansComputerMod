"""Item storage: Storage Cells in Drives, Storage Modules and on a Wired Bus Module.

    import storage

    net = storage.net()                     # everything this computer can reach
    for it in net.items("iron"):            # search by id or name ('@create' = one mod)
        print(it["name"], it["count"], it["key"])
    key = net.find("minecraft:iron_ingot")[0]
    net.extract(key, 16)                    # out of the first Item Decoder in reach

    cell = net.cells()[0]["id"]
    token = net.withdraw_token(cell, key, 32)          # items leave the cell...
    # ...send `token` to another computer (any way you like), which calls:
    net.redeem(token)                                  # ...and land in one of its cells

    ev = storage.wait_changed(timeout=5)    # ('storage_changed', attachment, [cell ids]) or None

Items are named by key ("k3f"): one per distinct item (item + components).
Cells by id (a UUID; any unique 8+ character prefix works). Every call acts
on the computer's whole storage net and runs on the server, one at a time:
the world's ledger is the only authority, so copying a token or the output
of items() anywhere copies nothing.

A token is single use: the first redeem wins, a second fails. Unspent tokens
expire (default 10 minutes of game time) and go back to their cell, or to
lost & found (see lost() / claim_lost()) if it's gone or full. Pass
to=<cell id> to address a token so only that cell can redeem it.
"""

import peripheral

TYPES = ("storage_drive", "storage_module", "wired_sensors")


class Net:
    """A computer's storage net, reached through one storage peripheral."""

    def __init__(self, p):
        self.peripheral = p

    def __repr__(self):
        return "<storage net via %s>" % self.peripheral.name

    # ---- inventory

    def cells(self):
        """[{id, short, tier, device, slot, bytes_used, bytes_total, types_used, types_total, items}]"""
        return self.peripheral.cells()

    def items(self, query=None, sort="count", offset=0, limit=0, cell=None):
        """[{key, id, name, mod, count}]. sort: count | name | id | mod."""
        return self.peripheral.items(query, sort, offset, limit, cell)

    def item_count(self, query=None, cell=None):
        return self.peripheral.item_count(query, cell)

    def total(self, key):
        """How many of an item (by key) are stored."""
        return self.peripheral.total(key)

    def find(self, item_id):
        """Keys of stored items with this id (several if they differ, e.g. by enchantment)."""
        return self.peripheral.find(item_id)

    def detail(self, key):
        """{key, id, name, count, max_stack, damage?, enchantments?, components}"""
        return self.peripheral.item_detail(key)

    def devices(self):
        """{'devices': [{id, kind, priority, where, cells}], 'ports': [{name, kind, id}]}"""
        return self.peripheral.devices()

    def decoders(self):
        """Names of the Item Decoders in reach."""
        return [p["name"] for p in self.devices()["ports"] if p["kind"] == "decoder"]

    # ---- moving items

    def move(self, from_cell, to_cell, key, count):
        """Move up to `count` between two cells; returns how many moved."""
        return self.peripheral.move(from_cell, to_cell, key, count)

    def extract(self, key, count, decoder=None, cell=None):
        """Send up to `count` out of a decoder (default: the first); returns how many."""
        return self.peripheral.extract(key, count, decoder, cell)

    # ---- tokens

    def withdraw_token(self, cell, key, count, to=None, ttl=None):
        """Take items out of `cell` as a single-use token string. `ttl` in seconds."""
        return self.peripheral.withdraw_token(cell, key, count, to, ttl)

    def redeem(self, token, cell=None):
        """Spend a token into `cell` (default: its address, else the first with room)."""
        return self.peripheral.redeem(token, cell)

    def token_info(self, token):
        """{key, id, name, count, expires_in, bound, issuer} without spending it."""
        return self.peripheral.token_info(token)

    def split(self, token, amounts):
        """Split a token into several (counts must add up); returns the new tokens."""
        return self.peripheral.split(token, list(amounts))

    def merge(self, tokens):
        """Merge tokens of the same item and origin into one."""
        return self.peripheral.merge(list(tokens))

    def tokens_issued(self):
        return self.peripheral.tokens_issued()

    def lost(self):
        return self.peripheral.lost()

    def claim_lost(self, cell=None):
        return self.peripheral.claim_lost(cell)


def net():
    """The storage net, through the first storage peripheral attached."""
    for t in TYPES:
        p = peripheral.find(t)
        if p is not None:
            return Net(p)
    raise peripheral.PeripheralError("no storage attached (a Drive, a Storage Module, or a Wired Bus Module)")


def encoder(name=None):
    """An Item Encoder next to the computer (by attachment name, or the first)."""
    return peripheral.wrap(name) if name else peripheral.find("item_encoder")


def decoder(name=None):
    """An Item Decoder next to the computer (by attachment name, or the first)."""
    return peripheral.wrap(name) if name else peripheral.find("item_decoder")


def wait_changed(timeout=None):
    """Wait for a storage_changed event: ('storage_changed', attachment, [cell ids]) or None."""
    return peripheral.pull_event("storage_changed", timeout)
