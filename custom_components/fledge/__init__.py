"""Fledge: Home Assistant on a Nest thermostat's own screen.

The thermostat (nestlocal) shows whatever cards are retained on
nestlocal/<serial>/set/ui, and asks for changes on nestlocal/<serial>/ui/
command. This keeps the cards up to date as the chosen devices change, and
carries out the commands: only for those devices, and only the ways
cards.command allows.
"""

from __future__ import annotations

import json
import logging
from typing import Any

from homeassistant.components import mqtt
from homeassistant.components.media_player import DATA_COMPONENT as MEDIA_PLAYERS
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import Event, HomeAssistant, callback
from homeassistant.exceptions import ConfigEntryNotReady
from homeassistant.helpers import area_registry as ar, device_registry as dr, entity_registry as er
from homeassistant.helpers.debounce import Debouncer
from homeassistant.helpers.event import async_track_state_change_event

from . import art, cards
from .const import (ART_TOPIC, CARDS_TOPIC, COMMAND_TOPIC, CONF_AREAS, CONF_CLIMATES, CONF_ENTITIES, CONF_EXCLUDE, CONF_MEDIA,
                    CONF_SERIAL, CONF_WAKE, CONF_WAKE_DEVICE, TOPIC_PREFIX, WAKE_CHOOSER)

_LOGGER = logging.getLogger(__name__)

type FledgeConfigEntry = ConfigEntry["Thermostat"]


async def async_setup_entry(hass: HomeAssistant, entry: FledgeConfigEntry) -> bool:
    if not await mqtt.async_wait_for_mqtt_client(hass):
        raise ConfigEntryNotReady("MQTT is not ready")
    if CONF_ENTITIES not in entry.options:
        _migrate(hass, entry)
    thermostat = Thermostat(hass, entry)
    await thermostat.async_start()
    entry.runtime_data = thermostat
    entry.async_on_unload(thermostat.async_stop)
    entry.async_on_unload(entry.add_update_listener(_reload))
    return True


async def async_unload_entry(hass: HomeAssistant, entry: FledgeConfigEntry) -> bool:
    return True


async def async_remove_entry(hass: HomeAssistant, entry: ConfigEntry) -> None:
    """No more cards: the thermostat goes back to how it was."""
    serial = entry.data[CONF_SERIAL]
    await mqtt.async_publish(hass, CARDS_TOPIC.format(prefix=TOPIC_PREFIX, serial=serial), "", retain=True)
    for entity in entry.options.get(CONF_ENTITIES, []):
        if entity.startswith("media_player."):
            await mqtt.async_publish(hass, ART_TOPIC.format(prefix=TOPIC_PREFIX, serial=serial, entity=entity), "",
                                     retain=True)


async def _reload(hass: HomeAssistant, entry: ConfigEntry) -> None:
    await hass.config_entries.async_reload(entry.entry_id)


def _area_lights(hass: HomeAssistant, area_id: str, left_out: set[str]) -> list[str]:
    """An area's lights by name: its own, and its devices' that are not
    moved elsewhere. Hidden is about dashboards (a light group often is):
    still listed."""
    devices, entities = dr.async_get(hass), er.async_get(hass)
    found = {e.entity_id: e for e in er.async_entries_for_area(entities, area_id)}
    for device in dr.async_entries_for_area(devices, area_id):
        for e in er.async_entries_for_device(entities, device.id):
            if e.area_id in (None, area_id):
                found.setdefault(e.entity_id, e)
    lights = [e for e in found.values() if e.domain == "light" and not e.disabled_by and e.entity_id not in left_out]
    lights.sort(key=lambda e: (e.name or e.original_name or e.entity_id).lower())
    return [e.entity_id for e in lights]


def _migrate(hass: HomeAssistant, entry: ConfigEntry) -> None:
    """From rooms of lights (before 0.4.4) to a card each: the rooms'
    lights as they are now, then the thermostats and media players."""
    old = entry.options
    left_out = set(old.get(CONF_EXCLUDE, []))
    entities: list[str] = []
    for area_id in old.get(CONF_AREAS, []):
        if ar.async_get(hass).async_get_area(area_id) is not None:
            entities += [e for e in _area_lights(hass, area_id, left_out) if e not in entities]
    entities += [*old.get(CONF_CLIMATES, []), *old.get(CONF_MEDIA, [])]
    wake = old.get(CONF_WAKE, WAKE_CHOOSER)
    options: dict[str, Any] = {CONF_ENTITIES: entities, CONF_WAKE: wake if wake in ("chooser", "thermostat") else WAKE_CHOOSER}
    if wake.startswith("entity:"):
        options[CONF_WAKE_DEVICE] = wake.removeprefix("entity:")
    elif wake.startswith("area:"):
        lights = _area_lights(hass, wake.removeprefix("area:"), left_out)
        if lights:
            options[CONF_WAKE_DEVICE] = lights[0]
    _LOGGER.info("Fledge: the cards are now a device each: %s", ", ".join(entities) or "none")
    hass.config_entries.async_update_entry(entry, options=options)


class Thermostat:
    """One Nest's cards: kept published, and its commands carried out."""

    def __init__(self, hass: HomeAssistant, entry: ConfigEntry) -> None:
        self.hass = hass
        self.entry = entry
        serial = entry.data[CONF_SERIAL]
        self.cards_topic = CARDS_TOPIC.format(prefix=TOPIC_PREFIX, serial=serial)
        self.command_topic = COMMAND_TOPIC.format(prefix=TOPIC_PREFIX, serial=serial)
        self.entities: list[str] = list(entry.options.get(CONF_ENTITIES, []))
        self.serial = serial
        # Each media player's picture as last sent: its entity_picture, or None.
        self.art: dict[str, str | None] = {}
        self.last: str | None = None
        self._unsubscribe: list[Any] = []
        self._tracking: Any = None
        self._publisher = Debouncer(hass, _LOGGER, cooldown=0.2, immediate=True, function=self._publish)

    async def async_start(self) -> None:
        if self.entities:
            self._tracking = async_track_state_change_event(self.hass, self.entities, self._changed)
        self._unsubscribe.append(await mqtt.async_subscribe(self.hass, self.command_topic, self._command))
        await self._publish()

    @callback
    def async_stop(self) -> None:
        for unsubscribe in self._unsubscribe:
            unsubscribe()
        if self._tracking:
            self._tracking()
        self._publisher.async_shutdown()

    @callback
    def _changed(self, event: Event) -> None:
        self.hass.async_create_task(self._publisher.async_call())

    async def _publish(self) -> None:
        options = self.entry.options
        payload = json.dumps(cards.cards(self.entities, self.hass.states.get, options.get(CONF_WAKE, WAKE_CHOOSER),
                                         options.get(CONF_WAKE_DEVICE)), separators=(",", ":"))
        if payload != self.last:
            self.last = payload
            await mqtt.async_publish(self.hass, self.cards_topic, payload, retain=True)
        for entity in self.entities:
            state = self.hass.states.get(entity)
            if not entity.startswith("media_player.") or state is None:
                continue
            picture = None if state.state in ("off", "unavailable") else state.attributes.get("entity_picture")
            if entity not in self.art or self.art[entity] != picture:
                self.art[entity] = picture
                self.hass.async_create_task(self._send_art(entity, picture))

    async def _send_art(self, entity: str, picture: str | None) -> None:
        """A media player's album art, shrunk for the thermostat, or none."""
        png: bytes | None = None
        player = self.hass.data[MEDIA_PLAYERS].get_entity(entity) if picture and MEDIA_PLAYERS in self.hass.data else None
        if player is not None:
            try:
                data, _ = await player.async_get_media_image()
            except Exception:  # noqa: BLE001 - a player failing to fetch its art: none shown
                _LOGGER.debug("No album art from %s", entity, exc_info=True)
                data = None
            if data:
                png = await self.hass.async_add_executor_job(art.thumbnail, data)
        if self.art.get(entity) != picture:
            return  # the track changed again meanwhile
        topic = ART_TOPIC.format(prefix=TOPIC_PREFIX, serial=self.serial, entity=entity)
        await mqtt.async_publish(self.hass, topic, png or b"", retain=True)

    @callback
    def _command(self, message: mqtt.ReceiveMessage) -> None:
        try:
            payload = json.loads(message.payload)
        except (TypeError, ValueError):
            return
        call = cards.command(payload, self.entities) if isinstance(payload, dict) else None
        if call is None:
            _LOGGER.warning("Ignoring a command from the thermostat it may not give: %s", message.payload)
            return
        self.hass.async_create_task(self.hass.services.async_call(
            call.domain, call.service, {"entity_id": call.entity_id, **call.data}, blocking=False))
