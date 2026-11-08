"""Fledge: setting a thermostat up, its cards, and what its commands do."""

import json

from homeassistant import config_entries
from homeassistant.core import HomeAssistant
from homeassistant.data_entry_flow import FlowResultType
from homeassistant.helpers import area_registry as ar, device_registry as dr, entity_registry as er
from homeassistant.helpers.service_info.mqtt import MqttServiceInfo
from pytest_homeassistant_custom_component.common import (
    MockConfigEntry,
    async_fire_mqtt_message,
    async_mock_service,
)

from custom_components.fledge import cards
from custom_components.fledge.const import DOMAIN

SERIAL = "09AB01AC36230301"
CARDS = f"nestlocal/{SERIAL}/set/ui"
COMMANDS = f"nestlocal/{SERIAL}/ui/command"


class State:
    def __init__(self, entity_id, state, **attributes):
        self.entity_id, self.state, self.attributes = entity_id, state, attributes


def test_a_light_as_the_thermostat_lists_it():
    lamp = State("light.lamp", "on", friendly_name="Lamp", icon="m3s:table-lamp", brightness=153,
                 supported_color_modes=["color_temp"], rgb_color=(255, 169, 87))
    assert cards.light(lamp) == {"id": "light.lamp", "name": "Lamp", "icon": "m3s:table-lamp", "on": True,
                                 "dimmable": True, "brightness": 60, "rgb": [255, 169, 87]}
    off = State("light.lamp", "off", supported_color_modes=["onoff"])
    assert cards.light(off)["brightness"] is None and not cards.light(off)["dimmable"]
    faint = State("light.lamp", "on", brightness=1, supported_color_modes=["brightness"])
    assert cards.light(faint)["brightness"] == 1, "on is at least 1%"


def test_the_thermostat_wakes_to_what_was_chosen():
    rooms = [cards.LightsCard("Bedroom", "mdi:bed", ["light.lamp"], "area:bedroom"),
             cards.LightsCard("Hall", "mdi:door", ["light.gone"], "area:hall")]
    states = {"light.lamp": State("light.lamp", "on"), "climate.utility": State("climate.utility", "heat")}
    wake = lambda w: cards.cards(rooms, ["climate.utility"], [], states.get, w)["wake"]
    assert wake("chooser") == "chooser" and wake("thermostat") == "thermostat"
    assert wake("area:bedroom") == 0 and wake("entity:climate.utility") == 1, "by its place among the cards"
    assert wake("area:hall") == "chooser", "a card that isn't there: the chooser"


def test_a_room_with_no_known_lights_has_no_card():
    rooms = [cards.LightsCard("Bedroom", "mdi:bed", ["light.gone"]),
             cards.LightsCard("Kitchen", "mdi:knife", ["light.kitchen"])]
    states = {"light.kitchen": State("light.kitchen", "off"),
              "climate.utility": State("climate.utility", "heat", temperature=21, hvac_action="idle"),
              "media_player.speaker": State("media_player.speaker", "playing", volume_level=0.35)}
    out = cards.cards(rooms, ["climate.utility"], ["media_player.speaker"], states.get)["cards"]
    assert [c["kind"] for c in out] == ["lights", "climate", "media"]
    assert out[0]["name"] == "Kitchen" and out[1]["target"] == 21 and out[2]["volume"] == 35


def test_commands_reach_only_what_was_chosen_only_as_allowed():
    lights, climates, players = ["light.lamp", "switch.fan"], ["climate.utility"], ["media_player.speaker"]
    command = lambda payload: cards.command(payload, lights, climates, players)
    assert command({"id": "light.lamp", "action": "brightness", "value": 40}) == \
        cards.Call("light", "turn_on", "light.lamp", {"brightness_pct": 40})
    assert command({"id": "switch.fan", "action": "on"}) == cards.Call("homeassistant", "turn_on", "switch.fan", {})
    assert command({"id": "climate.utility", "action": "temperature", "value": 20.5}).data == {"temperature": 20.5}
    assert command({"id": "media_player.speaker", "action": "volume", "value": 35}).data == {"volume_level": 0.35}
    refused = [
        {"id": "light.other", "action": "on"},
        {"id": "light.lamp", "action": "temperature", "value": 20},
        {"id": "switch.fan", "action": "brightness", "value": 50},
        {"id": "climate.utility", "action": "temperature", "value": 90},
        {"id": "light.lamp", "action": "brightness", "value": "lots"},
        {"id": ["light.lamp"], "action": "on"},
        {"action": "on"},
    ]
    assert [command(p) for p in refused] == [None] * len(refused)


def _thermostat(hass: HomeAssistant) -> dr.DeviceEntry:
    mqtt_entry = MockConfigEntry(domain="mqtt")
    mqtt_entry.add_to_hass(hass)
    return dr.async_get(hass).async_get_or_create(
        config_entry_id=mqtt_entry.entry_id, identifiers={("mqtt", f"nestlocal_{SERIAL.lower()}")},
        name="Nest Thermostat", manufacturer="Google Nest", serial_number=SERIAL)


async def test_choosing_the_thermostat_and_its_cards(hass: HomeAssistant, mqtt_mock) -> None:
    thermostat = _thermostat(hass)
    # Other MQTT devices, one with an identifier of three parts.
    dr.async_get(hass).async_get_or_create(config_entry_id=next(iter(thermostat.config_entries)),
                                           identifiers={("mqtt", "zigbee2mqtt_0x1", "extra")}, name="Other")
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_USER})
    assert result["type"] is FlowResultType.FORM and result["step_id"] == "user"
    result = await hass.config_entries.flow.async_configure(result["flow_id"], {"serial": SERIAL})
    assert result["step_id"] == "cards"
    result = await hass.config_entries.flow.async_configure(result["flow_id"], {"climates": [], "media": []})
    assert result["step_id"] == "wake"
    result = await hass.config_entries.flow.async_configure(result["flow_id"], {"wake": "thermostat"})
    assert result["type"] is FlowResultType.CREATE_ENTRY
    assert result["title"] == f"Nest Thermostat ({SERIAL})"
    assert result["data"] == {"serial": SERIAL}
    assert result["options"]["wake"] == "thermostat"

    entry = hass.config_entries.async_entries(DOMAIN)[0]
    result = await hass.config_entries.options.async_init(entry.entry_id)
    result = await hass.config_entries.options.async_configure(result["flow_id"], {"climates": [], "media": []})
    assert result["step_id"] == "wake"
    result = await hass.config_entries.options.async_configure(result["flow_id"], {"wake": "chooser"})
    assert result["type"] is FlowResultType.CREATE_ENTRY and entry.options["wake"] == "chooser"


async def test_without_a_thermostat_there_is_nothing_to_set_up(hass: HomeAssistant, mqtt_mock) -> None:
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_USER})
    assert result["type"] is FlowResultType.ABORT and result["reason"] == "no_thermostats"


async def test_a_thermostat_coming_online_is_offered(hass: HomeAssistant, mqtt_mock) -> None:
    info = MqttServiceInfo(topic=f"nestlocal/{SERIAL}/availability", payload="online", qos=0, retain=True,
                           subscribed_topic="nestlocal/+/availability", timestamp=0)
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_MQTT},
                                                       data=info)
    assert result["step_id"] == "confirm"
    result = await hass.config_entries.flow.async_configure(result["flow_id"], {})
    result = await hass.config_entries.flow.async_configure(result["flow_id"], {})
    result = await hass.config_entries.flow.async_configure(result["flow_id"], {})
    assert result["type"] is FlowResultType.CREATE_ENTRY and result["data"] == {"serial": SERIAL}
    assert result["options"]["wake"] == "chooser", "the chooser unless chosen"
    again = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_MQTT},
                                                      data=info)
    assert again["type"] is FlowResultType.ABORT and again["reason"] == "already_configured"
    other = MqttServiceInfo(topic="something/else", payload="online", qos=0, retain=True,
                            subscribed_topic="nestlocal/+/availability", timestamp=0)
    result = await hass.config_entries.flow.async_init(DOMAIN, context={"source": config_entries.SOURCE_MQTT},
                                                       data=other)
    assert result["reason"] == "not_fledge"


def _published(mqtt_mock) -> list:
    return [c.args for c in mqtt_mock.async_publish.call_args_list if c.args[0] == CARDS]


async def test_a_room_s_lights_are_kept_on_the_thermostat(hass: HomeAssistant, mqtt_mock) -> None:
    bedroom = ar.async_get(hass).async_create("Bedroom", icon="mdi:bed")
    entities = er.async_get(hass)
    for name in ("lamp", "ceiling", "screen"):
        entities.async_get_or_create("light", "test", name, suggested_object_id=name)
        entities.async_update_entity(f"light.{name}", area_id=bedroom.id)
    # A light group hidden from dashboards is still a room's light.
    entities.async_update_entity("light.ceiling", hidden_by=er.RegistryEntryHider.USER)
    hass.states.async_set("light.lamp", "on", {"friendly_name": "Lamp", "brightness": 255,
                                               "supported_color_modes": ["brightness"]})
    hass.states.async_set("light.ceiling", "off", {"friendly_name": "Ceiling"})
    entry = MockConfigEntry(domain=DOMAIN, unique_id=SERIAL, data={"serial": SERIAL},
                            options={"areas": [bedroom.id], "exclude": ["light.screen"], "climates": [], "media": []})
    entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(entry.entry_id)
    await hass.async_block_till_done()

    topic, payload, qos, retain = _published(mqtt_mock)[-1]
    card = json.loads(payload)["cards"][0]
    assert json.loads(payload)["wake"] == "chooser"
    assert retain and card["name"] == "Bedroom" and card["icon"] == "mdi:bed"
    assert [(l["name"], l["on"], l["brightness"]) for l in card["lights"]] == [("Ceiling", False, None),
                                                                                ("Lamp", True, 100)]

    hass.states.async_set("light.ceiling", "on", {"friendly_name": "Ceiling"})
    await hass.async_block_till_done()
    card = json.loads(_published(mqtt_mock)[-1][1])["cards"][0]
    assert card["lights"][0]["on"], "a light switched elsewhere shows on the thermostat"

    turn_on = async_mock_service(hass, "light", "turn_on")
    async_fire_mqtt_message(hass, COMMANDS, json.dumps({"id": "light.lamp", "action": "brightness", "value": 30}))
    async_fire_mqtt_message(hass, COMMANDS, json.dumps({"id": "light.kitchen", "action": "brightness", "value": 30}))
    async_fire_mqtt_message(hass, COMMANDS, "not json")
    await hass.async_block_till_done()
    assert [c.data for c in turn_on] == [{"entity_id": "light.lamp", "brightness_pct": 30}]

    await hass.config_entries.async_remove(entry.entry_id)
    await hass.async_block_till_done()
    assert _published(mqtt_mock)[-1] == (CARDS, "", 0, True), "removed: the thermostat back as it was"
