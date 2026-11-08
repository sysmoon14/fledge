"""Setting Fledge up: which thermostat, and which devices it shows."""

from __future__ import annotations

from typing import Any

import voluptuous as vol

from homeassistant.config_entries import ConfigEntry, ConfigFlow, ConfigFlowResult, OptionsFlow
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers import device_registry as dr, entity_registry as er, selector
from homeassistant.helpers.service_info.mqtt import MqttServiceInfo

from .const import (CONF_ENTITIES, CONF_SERIAL, CONF_WAKE, CONF_WAKE_DEVICE, DOMAIN, DOMAINS, TOPIC_PREFIX,
                    WAKE_CHOOSER, WAKE_THERMOSTAT)


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
    """The devices, a card each in the order given, and what it wakes to."""
    devices = selector.EntitySelector(selector.EntitySelectorConfig(
        domain=DOMAINS, multiple=True, reorder=True, exclude_entities=_own_entities(hass, serial)))
    wake = selector.SelectSelector(selector.SelectSelectorConfig(
        options=[WAKE_CHOOSER, WAKE_THERMOSTAT], translation_key="wake", mode=selector.SelectSelectorMode.LIST))
    device = selector.EntitySelector(selector.EntitySelectorConfig(domain=DOMAINS))
    return vol.Schema({
        vol.Optional(CONF_ENTITIES, description={"suggested_value": current.get(CONF_ENTITIES, [])}): devices,
        vol.Required(CONF_WAKE, default=current.get(CONF_WAKE, WAKE_CHOOSER)): wake,
        vol.Optional(CONF_WAKE_DEVICE, description={"suggested_value": current.get(CONF_WAKE_DEVICE)}): device,
    })


def _options(user_input: dict[str, Any]) -> dict[str, Any]:
    options = {CONF_ENTITIES: list(user_input.get(CONF_ENTITIES) or []),
               CONF_WAKE: user_input.get(CONF_WAKE, WAKE_CHOOSER)}
    if user_input.get(CONF_WAKE_DEVICE):
        options[CONF_WAKE_DEVICE] = user_input[CONF_WAKE_DEVICE]
    return options


def _title(hass: HomeAssistant, serial: str) -> str:
    device = thermostats(hass).get(serial)
    name = device and (device.name_by_user or device.name)
    return f"{name or 'Nest Thermostat'} ({serial})"


class FledgeConfigFlow(ConfigFlow, domain=DOMAIN):
    """A Nest found over MQTT or chosen from the list, then its devices."""

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
                                           data={CONF_SERIAL: self._serial}, options=_options(user_input))
        return self.async_show_form(step_id="cards", data_schema=_cards_schema(self.hass, self._serial, {}))

    @staticmethod
    @callback
    def async_get_options_flow(config_entry: ConfigEntry) -> OptionsFlow:
        return FledgeOptionsFlow()


class FledgeOptionsFlow(OptionsFlow):
    """Changing which devices the thermostat shows, and what it wakes to."""

    async def async_step_init(self, user_input: dict[str, Any] | None = None) -> ConfigFlowResult:
        if user_input is not None:
            return self.async_create_entry(data=_options(user_input))
        serial = self.config_entry.data[CONF_SERIAL]
        return self.async_show_form(step_id="init",
                                    data_schema=_cards_schema(self.hass, serial, dict(self.config_entry.options)))
