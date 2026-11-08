"""Setting Fledge up: which thermostat, and which cards it shows."""

from __future__ import annotations

from typing import Any

import voluptuous as vol

from homeassistant.config_entries import ConfigEntry, ConfigFlow, ConfigFlowResult, OptionsFlow
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers import device_registry as dr, entity_registry as er, selector
from homeassistant.helpers.service_info.mqtt import MqttServiceInfo

from .const import CONF_AREAS, CONF_CLIMATES, CONF_EXCLUDE, CONF_MEDIA, CONF_SERIAL, DOMAIN, TOPIC_PREFIX


def thermostats(hass: HomeAssistant) -> dict[str, dr.DeviceEntry]:
    """Nests running Fledge that Home Assistant knows, by serial: the devices
    nestlocal announces over MQTT discovery."""
    found = {}
    devices = dr.async_get(hass)
    for mqtt_entry in hass.config_entries.async_entries("mqtt"):
        for device in dr.async_entries_for_config_entry(devices, mqtt_entry.entry_id):
            # An identifier is (domain, id), or longer for some integrations.
            for identifier in device.identifiers:
                if len(identifier) >= 2 and identifier[0] == "mqtt" and str(identifier[1]).startswith("nestlocal_"):
                    serial = device.serial_number or str(identifier[1]).removeprefix("nestlocal_").upper()
                    found[serial] = device
    return found


def _own_entities(hass: HomeAssistant, serial: str) -> list[str]:
    """The thermostat's own entities: not offered as other thermostats."""
    device = thermostats(hass).get(serial)
    if device is None:
        return []
    return [e.entity_id for e in er.async_entries_for_device(er.async_get(hass), device.id)]


def _cards_schema(hass: HomeAssistant, serial: str, current: dict[str, Any]) -> vol.Schema:
    own = _own_entities(hass, serial)
    return vol.Schema({
        vol.Optional(CONF_AREAS, default=current.get(CONF_AREAS, [])):
            selector.AreaSelector(selector.AreaSelectorConfig(multiple=True)),
        vol.Optional(CONF_EXCLUDE, default=current.get(CONF_EXCLUDE, [])):
            selector.EntitySelector(selector.EntitySelectorConfig(domain="light", multiple=True)),
        vol.Optional(CONF_CLIMATES, default=current.get(CONF_CLIMATES, [])):
            selector.EntitySelector(selector.EntitySelectorConfig(domain="climate", multiple=True,
                                                                  exclude_entities=own)),
        vol.Optional(CONF_MEDIA, default=current.get(CONF_MEDIA, [])):
            selector.EntitySelector(selector.EntitySelectorConfig(domain="media_player", multiple=True)),
    })


def _title(hass: HomeAssistant, serial: str) -> str:
    device = thermostats(hass).get(serial)
    name = device and (device.name_by_user or device.name)
    return f"{name or 'Nest Thermostat'} ({serial})"


class FledgeConfigFlow(ConfigFlow, domain=DOMAIN):
    """A Nest found over MQTT or chosen from the list, then its cards."""

    VERSION = 1

    def __init__(self) -> None:
        self._serial: str | None = None

    async def async_step_mqtt(self, discovery_info: MqttServiceInfo) -> ConfigFlowResult:
        """A Nest running Fledge came online: nestlocal/<serial>/availability."""
        parts = discovery_info.topic.split("/")
        if len(parts) != 3 or parts[0] != TOPIC_PREFIX or not parts[1]:
            return self.async_abort(reason="not_fledge")
        self._serial = parts[1]
        await self.async_set_unique_id(self._serial)
        self._abort_if_unique_id_configured()
        self.context["title_placeholders"] = {"serial": self._serial}
        return await self.async_step_confirm()

    async def async_step_confirm(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        if user_input is not None:
            return await self.async_step_cards()
        return self.async_show_form(step_id="confirm", description_placeholders={"serial": self._serial or ""})

    async def async_step_user(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        configured = {entry.unique_id for entry in self._async_current_entries()}
        choices = {serial: _title(self.hass, serial)
                   for serial in thermostats(self.hass) if serial not in configured}
        if not choices:
            return self.async_abort(reason="no_thermostats")
        if user_input is not None:
            self._serial = user_input[CONF_SERIAL]
            await self.async_set_unique_id(self._serial)
            self._abort_if_unique_id_configured()
            return await self.async_step_cards()
        return self.async_show_form(step_id="user", data_schema=vol.Schema({
            vol.Required(CONF_SERIAL): vol.In(choices),
        }))

    async def async_step_cards(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        assert self._serial is not None
        if user_input is not None:
            return self.async_create_entry(title=_title(self.hass, self._serial),
                                           data={CONF_SERIAL: self._serial}, options=user_input)
        return self.async_show_form(step_id="cards", data_schema=_cards_schema(self.hass, self._serial, {}))

    @staticmethod
    @callback
    def async_get_options_flow(config_entry: ConfigEntry) -> OptionsFlow:
        return FledgeOptionsFlow()


class FledgeOptionsFlow(OptionsFlow):
    """Changing which cards the thermostat shows."""

    async def async_step_init(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        if user_input is not None:
            return self.async_create_entry(data=user_input)
        serial = self.config_entry.data[CONF_SERIAL]
        return self.async_show_form(step_id="init",
                                    data_schema=_cards_schema(self.hass, serial, dict(self.config_entry.options)))
