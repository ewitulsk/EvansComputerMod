"""16px textures for the Lidar Sensor, Sensor Wire (item + in-world wire)."""
import os
from PIL import Image

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'src', 'main', 'resources',
                   'assets', 'evanscomputermod', 'textures')


def hexc(h, a=255):
    h = h.lstrip('#')
    return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), a)


def canvas(fill=(0, 0, 0, 0)):
    return Image.new('RGBA', (16, 16), fill)


def rect(im, x0, y0, x1, y1, c):
    for y in range(y0, y1):
        for x in range(x0, x1):
            im.putpixel((x, y), c)


def px(im, x, y, c):
    im.putpixel((x, y), c)


# ---------------------------------------------------------------- lidar atlas
# u,v layout (pixels):
#   0..6 x 0..6    base plate top      (6x6)
#   0..6 x 6..7    base plate edge     (6x1)
#   6..10 x 0..3   head side, window   (4x3)
#   6..10 x 3..7   head top / bottom   (4x4)
#   10..12 x 0..2  connector (brass)
#   10..14 x 2..4  shelf / arm metal
#   0..6 x 8..16   wall plate          (6x8)
#   12..16 x 0..2  dark metal (plate back)
BODY, BODY_HI, BODY_LO = hexc('4b5058'), hexc('6a7079'), hexc('33373d')
im = canvas()
rect(im, 0, 0, 6, 6, BODY)
rect(im, 0, 0, 6, 1, BODY_HI); rect(im, 0, 0, 1, 6, BODY_HI)
rect(im, 0, 5, 6, 6, BODY_LO); rect(im, 5, 0, 6, 6, BODY_LO)
for x, y in ((1, 1), (4, 1), (1, 4), (4, 4)):
    px(im, x, y, hexc('9aa0a8'))            # screws
rect(im, 0, 6, 6, 7, BODY_LO)
# head side: dark housing, glass window band with a cyan emitter in the middle
rect(im, 6, 0, 10, 3, hexc('23272c'))
rect(im, 6, 1, 10, 2, hexc('0c1a22'))
px(im, 7, 1, hexc('1e5360')); px(im, 8, 1, hexc('58f0ff'))
rect(im, 6, 0, 10, 1, hexc('393e45'))
# head top: cap with a ring
rect(im, 6, 3, 10, 7, hexc('2d3238'))
rect(im, 7, 4, 9, 6, hexc('454b52'))
px(im, 7, 4, hexc('5d646c'))
rect(im, 10, 0, 12, 2, hexc('c9a13a'))
px(im, 10, 1, hexc('8e6f22'))
rect(im, 10, 2, 14, 4, BODY)
rect(im, 10, 2, 14, 3, BODY_HI)
rect(im, 0, 8, 6, 16, BODY)
rect(im, 0, 8, 6, 9, BODY_HI); rect(im, 0, 15, 6, 16, BODY_LO)
for x, y in ((1, 9), (4, 9), (1, 14), (4, 14)):
    px(im, x, y, hexc('9aa0a8'))
rect(im, 12, 0, 16, 2, BODY_LO)
im.save(OUT + '/block/lidar_sensor.png')

# ---------------------------------------------------------------- lidar item (flat icon)
ic = canvas()
rect(ic, 3, 12, 13, 14, BODY_LO)
rect(ic, 3, 11, 13, 12, BODY)
rect(ic, 4, 5, 12, 11, hexc('23272c'))
rect(ic, 4, 4, 12, 5, hexc('393e45'))
rect(ic, 4, 7, 12, 9, hexc('0c1a22'))
px(ic, 7, 7, hexc('58f0ff')); px(ic, 8, 7, hexc('58f0ff')); px(ic, 7, 8, hexc('2aa0b0')); px(ic, 8, 8, hexc('2aa0b0'))
px(ic, 5, 8, hexc('1e5360')); px(ic, 10, 8, hexc('1e5360'))
rect(ic, 12, 12, 14, 13, hexc('c9a13a'))
ic.save(OUT + '/item/lidar_sensor.png')

# ---------------------------------------------------------------- in-world wire (tinted by the wire colour)
os.makedirs(OUT + '/entity', exist_ok=True)
w = canvas()
for y in range(16):
    for x in range(16):
        v = 200 + ((x * 7 + y * 3) % 5) * 8
        if (x + y) % 8 == 0:
            v = 180
        w.putpixel((x, y), (v, v, v, 255))
w.save(OUT + '/entity/sensor_wire.png')

# ---------------------------------------------------------------- wire item (a coil)
coil = canvas()
cu, cu_hi, cu_lo = hexc('4a4f57'), hexc('6f7680'), hexc('2c3036')
for (x0, y0, x1, y1) in ((3, 4, 13, 12),):
    rect(coil, x0, y0, x1, y1, cu)
rect(coil, 5, 6, 11, 10, (0, 0, 0, 0))
for x in range(3, 13):
    px(coil, x, 4, cu_hi); px(coil, x, 11, cu_lo)
for y in range(4, 12):
    px(coil, 3, y, cu_hi); px(coil, 12, y, cu_lo)
for x in range(5, 11):
    px(coil, x, 5, cu_lo)
rect(coil, 12, 11, 14, 12, hexc('c9a13a'))
px(coil, 14, 12, hexc('c9a13a'))
px(coil, 15, 13, hexc('8e6f22'))
coil.save(OUT + '/item/sensor_wire.png')
print('sensor textures ok')
