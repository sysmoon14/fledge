"""Fledge: Home Assistant on a Nest thermostat's own screen.

The thermostat (nestlocal) shows whatever cards are retained on
nestlocal/<serial>/set/ui, and asks for changes on nestlocal/<serial>/ui/
command. This keeps the cards up to date as the chosen rooms' lights,
thermostats and media players change, and carries out the commands: only
for those entities, and only the ways cards.command allows.
"""

from __future__ import annotations

import json
import logging
from typing import Any

from homeassistant.components import mqtt
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import Event, HomeAssistant, callback
from homeassistant.exceptions import ConfigEntryNotReady
from homeassistant.helpers import area_registry as ar, device_registry as dr, entity_registry as er
from homeassistant.helpers.debounce import Debouncer
from homeassistant.helpers.event import async_track_state_change_event

from . import cards
from .const import (CARDS_TOPIC, COMMAND_TOPIC, CONF_AREAS, CONF_CLIMATES, CONF_EXCLUDE, CONF_MEDIA,
                    CONF_SERIAL, TOPIC_PREFIX)

_LOGGER = logging.getLogger(__name__)

type FledgeConfigEntry = ConfigEntry["Thermostat"]


async def async_setup_entry(hass: HomeAssistant, entry: FledgeConfigEntry) -> bool:
    if not await mqtt.async_wait_for_mqtt_client(hass):
        raise ConfigEntryNotReady("MQTT is not ready")
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
    topic = CARDS_TOPIC.format(prefix=TOPIC_PREFIX, serial=entry.data[CONF_SERIAL])
    await mqtt.async_publish(hass, topic, "", retain=True)


async def _reload(hass: HomeAssistant, entry: ConfigEntry) -> None:
    await hass.config_entries.async_reload(entry.entry_id)


class Thermostat:
    """One Nest's cards: kept published, and its commands carried out."""

    def __init__(self, hass: HomeAssistant, entry: ConfigEntry) -> None:
        self.hass = hass
        self.entry = entry
        serial = entry.data[CONF_SERIAL]
        self.cards_topic = CARDS_TOPIC.format(prefix=TOPIC_PREFIX, serial=serial)
        self.command_topic = COMMAND_TOPIC.format(prefix=TOPIC_PREFIX, serial=serial)
        self.rooms: list[cards.LightsCard] = []
        self.climates: list[str] = list(entry.options.get(CONF_CLIMATES, []))
        self.players: list[str] = list(entry.options.get(CONF_MEDIA, []))
        self.last: str | None = None
        self._unsubscribe: list[Any] = []
        self._tracking: Any = None
        self._publisher = Debouncer(hass, _LOGGER, cooldown=0.2, immediate=True, function=self._publish)

    @property
    def lights(self) -> list[str]:
        return [light for room in self.rooms for light in room.lights]

    async def async_start(self) -> None:
        self._find_rooms()
        self._track()
        self._unsubscribe.append(await mqtt.async_subscribe(self.hass, self.command_topic, self._command))
        # A room's lights change as lights move between areas.
        for event in (er.EVENT_ENTITY_REGISTRY_UPDATED, dr.EVENT_DEVICE_REGISTRY_UPDATED,
                      ar.EVENT_AREA_REGISTRY_UPDATED):
            self._unsubscribe.append(self.hass.bus.async_listen(event, self._registry_changed))
        await self._publish()

    @callback
    def async_stop(self) -> None:
        for unsubscribe in self._unsubscribe:
            unsubscribe()
        if self._tracking:
            self._tracking()
        self._publisher.async_shutdown()

    def _find_rooms(self) -> None:
        """Each chosen area's lights, on its own card: the area's name and
        icon, its lights by name."""
        areas, devices, entities = ar.async_get(self.hass), dr.async_get(self.hass), er.async_get(self.hass)
        rooms = []
        left_out = set(self.entry.options.get(CONF_EXCLUDE, []))
        for area_id in self.entry.options.get(CONF_AREAS, []):
            area = areas.async_get_area(area_id)
            if area is None:
                continue
            found = {e.entity_id: e for e in er.async_entries_for_area(entities, area_id)}
            for device in dr.async_entries_for_area(devices, area_id):
                for e in er.async_entries_for_device(entities, device.id):
                    if e.area_id in (None, area_id):
                        found.setdefault(e.entity_id, e)
            lights = [e for e in found.values() if e.domain == "light" and not e.disabled_by
                      and not e.hidden_by and e.entity_id not in left_out]
            lights.sort(key=lambda e: (e.name or e.original_name or e.entity_id).lower())
            rooms.append(cards.LightsCard(area.name, area.icon or "mdi:lightbulb-group",
                                          [e.entity_id for e in lights]))
        self.rooms = rooms

    def _track(self) -> None:
        if self._tracking:
            self._tracking()
        watched = self.lights + self.climates + self.players
        self._tracking = async_track_state_change_event(self.hass, watched, self._changed) if watched else None

    @callback
    def _changed(self, event: Event) -> None:
        self.hass.async_create_task(self._publisher.async_call())

    @callback
    def _registry_changed(self, event: Event) -> None:
        self._find_rooms()
        self._track()
        self.hass.async_create_task(self._publisher.async_call())

    async def _publish(self) -> None:
        payload = json.dumps(cards.cards(self.rooms, self.climates, self.players, self.hass.states.get),
                             separators=(",", ":"))
        if payload != self.last:
            self.last = payload
            await mqtt.async_publish(self.hass, self.cards_topic, payload, retain=True)

    @callback
    def _command(self, message: mqtt.ReceiveMessage) -> None:
        try:
            payload = json.loads(message.payload)
        except (TypeError, ValueError):
            return
        call = cards.command(payload, self.lights, self.climates, self.players) if isinstance(payload, dict) else None
        if call is None:
            _LOGGER.warning("Ignoring a command from the thermostat it may not give: %s", message.payload)
            return
        self.hass.async_create_task(self.hass.services.async_call(
            call.domain, call.service, {"entity_id": call.entity_id, **call.data}, blocking=False))
