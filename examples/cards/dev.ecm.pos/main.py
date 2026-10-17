"""pos: a toy point of sale (docs/cards/CARDS.md "Bank chip", "NFC applet").

    python /apps/dev.ecm.pos/main.py <bankd-host:port> <amount> [--nfc-key HEX] [--once]

Waits for a card, charges `amount` through `bankd` and prints APPROVED or
DECLINED. Two ways a card can pay:

  chip (contact): the card in the attached card_reader (block or bay module).
      The POS draws a fresh 8-byte nonce, asks the chip to sign (nonce, amount)
      -- the chip bumps its counter and returns {ctr, tag} -- and posts
      {pan, nonce, ctr, tag, amount}. The POS never sees the card's key; a
      recording of this exchange is worthless (bankd rejects an old counter).
      A card issued with a PIN asks for it first (three wrong tries lock it).

  tap (NFC): a card in an attached nfc_reader's field (a block, or a module
      in a Terminal or ePhone bay). The POS polls, selects the card,
      authenticates sector 1 with the issuer's terminal key (--nfc-key, from
      `bankd issue --tap`), reads the PAN and tap token stored there and posts
      {pan, token, amount, tap: true}. Nothing is signed: the token is a
      static secret read over the air, so an SDR in the field (`rx_nfc`) or a
      cloned sector pays with it too. That is the lesson; the chip path is the
      one to trust.

Without --nfc-key only the chip path is used.
"""

import sys

import cards
import peripheral


def _json():
    try:
        import _ui
        return _ui.json_dumps, _ui.json_loads
    except Exception:
        import json
        return json.dumps, json.loads


json_dumps, json_loads = _json()


def post(host_port, req):
    """One request to bankd: a JSON line in, a JSON line out."""
    import net
    host, _, port = host_port.rpartition(":")
    conn = net.tcp_connect(host, int(port))
    try:
        net.tcp_send(conn, (json_dumps(req) + "\n").encode())
        data = b""
        while b"\n" not in data and len(data) < 4096:
            chunk = net.tcp_recv(conn, 1024)
            if not chunk:
                break
            data += chunk
    finally:
        net.tcp_close(conn)
    return json_loads(data.decode().strip() or "{}")


def chip_request(card, amount, pin=None):
    """Build a chip payment request from the card in a reader (the signing step)."""
    ident = card.identity()
    bank = ident.get("bank") or {}
    pan = bank.get("pan")
    if not pan or bank.get("blank"):
        raise ValueError("not an issued bank card")
    if bank.get("locked"):
        raise ValueError("the card is locked")
    if pin is not None and not card.bank.verify_pin(pin):
        raise ValueError("wrong PIN")
    nonce = cards.random_hex(8)
    ctr, tag = card.bank.sign(nonce, amount)
    return {"pan": pan, "nonce": nonce, "ctr": ctr, "tag": tag, "amount": int(amount)}


def tap_request(nfc, uid, terminal_key, amount):
    """Build a tap request from a card in an NFC reader's field: sector 1 = PAN 00 token."""
    s = nfc.session(uid, 1, terminal_key)
    data = bytes.fromhex(s.read())
    end = data.find(b"\x00")
    pan = data[:end if end >= 0 else 19].decode()
    token = data[end + 1:end + 17].hex() if end >= 0 else ""
    if not pan.isdigit():
        raise ValueError("sector 1 of %s holds no PAN" % uid)
    return {"pan": pan, "token": token, "amount": int(amount), "tap": True}


def ask_pin():
    try:
        import shell
        return shell.input("PIN: ").strip()
    except Exception:
        return input("PIN: ").strip()


def wait_for_payment(reader, nfc, amount, terminal_key, timeout=120):
    """Block until a card pays (chip or tap) and return the request to post."""
    waited = 0.0
    while waited < timeout:
        if reader is not None and reader.present():
            card = reader.card()
            ident = card.identity()
            bank = ident.get("bank") or {}
            if bank.get("pan") and not bank.get("blank"):
                try:
                    try:
                        return chip_request(card, amount)
                    except peripheral.PeripheralError as e:
                        # The issuer set pin_required: the chip signs only inside a PIN session.
                        if "pin" not in str(e).lower():
                            raise
                        return chip_request(card, amount, ask_pin())
                finally:
                    reader.eject()
            # Not a bank card: let it go and keep waiting.
            reader.eject()
        if nfc is not None and terminal_key:
            for seen in nfc.poll():
                try:
                    return tap_request(nfc, seen["uid"], terminal_key, amount)
                except (peripheral.PeripheralError, ValueError) as e:
                    print("tap from %s refused: %s" % (seen["uid"], e))
        peripheral.pull_event(None, timeout=0.5)
        waited += 0.5
    return None


def main(argv):
    args = list(argv)
    key = None
    if "--nfc-key" in args:
        i = args.index("--nfc-key")
        key = args[i + 1]
        del args[i:i + 2]
    once = "--once" in args
    if once:
        args.remove("--once")
    if len(args) < 2:
        print(__doc__)
        return 2
    host_port, amount = args[0], int(args[1])
    reader = cards.Reader.find()
    nfc = cards.Nfc.find() if key else None
    if reader is None and nfc is None:
        print("pos: attach a card_reader (chip) and/or an nfc_reader with --nfc-key (tap)")
        return 1
    print("pos: charging %d via %s; %s%s" % (amount, host_port,
                                             "chip in " + reader.name if reader else "no chip reader",
                                             ", tap on " + nfc.name if nfc else ""))
    while True:
        print("present a card...")
        try:
            req = wait_for_payment(reader, nfc, amount, key)
        except (peripheral.PeripheralError, ValueError) as e:
            print("DECLINED: %s" % e)
            if once:
                return 1
            continue
        if req is None:
            print("pos: no card")
            return 1
        try:
            reply = post(host_port, req)
        except RuntimeError as e:
            print("DECLINED: bankd unreachable (%s)" % e)
            if once:
                return 1
            continue
        if reply.get("ok"):
            print("APPROVED %s %d (balance %s)" % (req["pan"], amount, reply.get("balance")))
        else:
            print("DECLINED %s %d: %s" % (req["pan"], amount, reply.get("reason")))
        if once:
            return 0 if reply.get("ok") else 1


if not globals().get("_TESTING"):
    main(list(sys.argv[1:]))
