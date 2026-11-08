"""The thermostat's cards, and what its commands do: no Home Assistant
imports, so it can be tested on its own.

The cards are what nestlocal reads on <base>/set/ui (crates/nestlocal/src/ui/
model.rs); a command is what it sends on <base>/ui/command:

    {"id": "light.x", "action": "on" | "off" | "brightness" | "temperature"
                                | "volume" | "play_pause", "value": <number>}
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping, Protocol, Sequence

from .const import MAX_LIGHTS


class State(Protocol):
    """What cards need of a Home Assistant state."""

    entity_id: str
    state: str
    attributes: Mapping[str, Any]


@dataclass
class LightsCard:
    """A room's lights: an area's name and icon, and its lights."""

    name: str
    icon: str
    lights: Sequence[str]
    # What the wake option calls this card: "area:<id>".
    key: str = ""


def _name(state: State) -> str:
    return state.attributes.get("friendly_name") or state.entity_id


def light(state: State) -> dict[str, Any]:
    """One light as the thermostat lists it: dimmable if it supports any
    colour mode beyond on/off; brightness in percent, at least 1 while on;
    its colour Home Assistant's rgb_color, which colour-temperature lights
    have too."""
    on = state.state == "on"
    modes = state.attributes.get("supported_color_modes") or []
    level = state.attributes.get("brightness")
    rgb = state.attributes.get("rgb_color")
    return {
        "id": state.entity_id,
        "name": _name(state),
        "icon": state.attributes.get("icon") or "",
        "on": on,
        "dimmable": state.entity_id.startswith("light.")
        and any(mode not in ("onoff", "unknown") for mode in modes),
        "brightness": max(round(level * 100 / 255), 1) if on and isinstance(level, (int, float)) else None,
        "rgb": list(rgb) if rgb else None,
    }


def climate(state: State) -> dict[str, Any]:
    attributes = state.attributes
    return {
        "kind": "climate",
        "id": state.entity_id,
        "name": _name(state),
        "icon": attributes.get("icon") or "",
        "mode": state.state,
        "action": attributes.get("hvac_action") or "",
        "target": attributes.get("temperature"),
        "current": attributes.get("current_temperature"),
        "min": attributes.get("min_temp") or 7,
        "max": attributes.get("max_temp") or 30,
        "step": attributes.get("target_temp_step") or 0.5,
    }


def media(state: State) -> dict[str, Any]:
    volume = state.attributes.get("volume_level")
    return {
        "kind": "media",
        "id": state.entity_id,
        "name": _name(state),
        "icon": state.attributes.get("icon") or "",
        "state": state.state,
        "title": state.attributes.get("media_title") or "",
        "artist": state.attributes.get("media_artist") or "",
        "volume": round(volume * 100) if isinstance(volume, (int, float)) else None,
    }


def cards(
    rooms: Sequence[LightsCard],
    climates: Sequence[str],
    players: Sequence[str],
    get: Any,
    wake: str = "chooser",
) -> dict[str, Any]:
    """The cards, from the chosen rooms, thermostats and media players, each
    entity's state from `get` (None: unknown to Home Assistant, left out),
    and what the thermostat wakes to: "chooser", "thermostat", or the place
    among the cards of the one `wake` names ("area:<id>", "entity:<id>"),
    the chooser if it isn't there."""
    out: list[dict[str, Any]] = []
    keys: list[str] = []
    for room in rooms:
        lights = [light(s) for s in map(get, room.lights) if s is not None][:MAX_LIGHTS]
        if lights:
            out.append({"kind": "lights", "name": room.name, "icon": room.icon, "lights": lights})
            keys.append(room.key)
    for build, entities in ((climate, climates), (media, players)):
        for state in map(get, entities):
            if state is not None:
                out.append(build(state))
                keys.append(f"entity:{state.entity_id}")
    woken: Any = "thermostat" if wake == "thermostat" else "chooser"
    if wake in keys:
        woken = keys.index(wake)
    return {"cards": out, "wake": woken}


@dataclass
class Call:
    """A Home Assistant action to carry out."""

    domain: str
    service: str
    entity_id: str
    data: dict[str, Any]


def command(payload: Mapping[str, Any], lights: Sequence[str], climates: Sequence[str],
            players: Sequence[str]) -> Call | None:
    """What a command from the thermostat does, or None if it isn't one it
    may give: only the chosen entities, only these ways."""
    entity = payload.get("id")
    what = payload.get("action")
    try:
        value = float(payload.get("value", 0))
    except (TypeError, ValueError):
        return None
    if not isinstance(entity, str):
        return None
    if entity in lights:
        if what == "off":
            return Call("homeassistant", "turn_off", entity, {})
        if what == "on":
            return Call("homeassistant", "turn_on", entity, {})
        if what == "brightness" and entity.startswith("light.") and 1 <= value <= 100:
            return Call("light", "turn_on", entity, {"brightness_pct": round(value)})
    if entity in climates and what == "temperature" and 5 <= value <= 35:
        return Call("climate", "set_temperature", entity, {"temperature": value})
    if entity in players:
        if what == "volume" and 0 <= value <= 100:
            return Call("media_player", "volume_set", entity, {"volume_level": value / 100})
        if what == "play_pause":
            return Call("media_player", "media_play_pause", entity, {})
    return None
