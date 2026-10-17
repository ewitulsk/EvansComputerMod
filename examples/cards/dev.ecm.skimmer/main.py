"""skimmer: copy a magnetic stripe (docs/cards/CARDS.md "Magnetic stripe").

    python /apps/dev.ecm.skimmer/main.py [--from NAME] [--to NAME]

The lesson: a stripe is plain text. Nothing on it is secret, nothing on it
is signed and nothing on it counts; whoever can read it can write an
identical one onto a blank. A program that trusts a stripe trusts anyone
who ever held the card (the bank's chip and the NFC sectors are the parts
that can't be copied this way).

With two readers (--from / --to, or the first two card_reader peripherals in
attachment order) the stripe of the card in A is written to the card in B.
With one reader, the first card's stripe is kept, and written to the next
card inserted. The tracks are printed either way: that's all a stripe is.
"""

import sys

import cards


def _opt(args, name):
    if name in args:
        i = args.index(name)
        v = args[i + 1]
        del args[i:i + 2]
        return v
    return None


def clone(src_card, dst_card):
    """Read the stripe of one card and write it onto another. Returns the tracks."""
    tracks = src_card.stripe.read()
    if tracks is None:
        raise ValueError("the source card has no stripe")
    if src_card.stripe.corrupted():
        print("note: the source stripe is corrupted (a strong field scrambled it); copying it anyway")
    dst_card.stripe.write(*tracks)
    return tracks


def wait_card(reader, what):
    if reader.present():
        return reader.card()
    print("insert %s into %s..." % (what, reader.name))
    ev = reader.wait(120)
    if not ev or ev[0] != "card_inserted":
        return None
    return reader.card()


def main(argv):
    args = list(argv)
    a_name = _opt(args, "--from")
    b_name = _opt(args, "--to")
    readers = cards.Reader.find_all()
    if a_name or b_name:
        a = cards.Reader.find(a_name) if a_name else (readers[0] if readers else None)
        b = cards.Reader.find(b_name) if b_name else (readers[1] if len(readers) > 1 else None)
    else:
        a = readers[0] if readers else None
        b = readers[1] if len(readers) > 1 else None
    if a is None:
        print("skimmer: no card_reader attached")
        return 1
    src = wait_card(a, "the card to copy")
    if src is None:
        print("skimmer: no card")
        return 1
    tracks = src.stripe.read()
    if tracks is None:
        print("skimmer: the card in %s has no stripe (an nfc or sim card)" % a.name)
        return 1
    print("read from %s:" % a.name)
    for i, t in enumerate(tracks):
        print("  track %d: %r" % (i + 1, t))
    if b is not None:
        dst = wait_card(b, "a blank with a stripe")
        if dst is None:
            print("skimmer: no blank")
            return 1
        dst.stripe.write(*tracks)
        print("written to the card in %s: an identical stripe" % b.name)
        return 0
    # One reader: keep the tracks, wait for the next card.
    a.eject()
    print("take the card out of %s and insert the blank..." % a.name)
    for _ in range(2):
        ev = a.wait(120)
        if ev and ev[0] == "card_inserted":
            break
    else:
        print("skimmer: no blank")
        return 1
    dst = a.card()
    dst.stripe.write(*tracks)
    print("written to the next card in %s: an identical stripe" % a.name)
    return 0


if not globals().get("_TESTING"):
    main(list(sys.argv[1:]))
