"""Album art for the thermostat: a media player's picture, square and small
enough for its card, as the PNG nestlocal reads on <base>/set/art/<entity>."""

from __future__ import annotations

from io import BytesIO

# How wide the thermostat shows it (nestlocal takes up to 240).
SIZE = 168


def thumbnail(data: bytes, size: int = SIZE) -> bytes | None:
    """The picture cropped square from its middle and scaled to `size`, as
    an RGB PNG; None if it isn't a picture Pillow reads."""
    try:
        from PIL import Image, ImageOps
    except ImportError:
        return None
    try:
        with Image.open(BytesIO(data)) as image:
            fitted = ImageOps.fit(image.convert("RGB"), (size, size), Image.Resampling.LANCZOS)
    except (OSError, ValueError):
        return None
    out = BytesIO()
    fitted.save(out, "PNG", optimize=True)
    return out.getvalue()
