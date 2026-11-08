# Fledge for Home Assistant

Home Assistant on the screen of a Nest Learning Thermostat (3rd generation)
running [Fledge](https://github.com/sysmoon14/fledge). Waking the thermostat
shows it among a card for each device you choose (lights, switches, fans,
other thermostats, media players), zoomed out: turning browses, pressing
opens one. A light dims, a switch switches, a thermostat sets its
temperature, a speaker its volume, with its album art on its card. A long
press switches a light on or off, or plays or pauses a speaker, from its card
or its own screen. A light group makes a whole room one card.

## Install

1. The thermostat runs Fledge and is connected to Home Assistant over MQTT
   (it shows up as a "Nest Thermostat" device).
2. In HACS, add `https://github.com/sysmoon14/fledge` as a custom repository
   of type Integration, and download Fledge. Restart Home Assistant.
3. Home Assistant offers the thermostat under Settings → Devices & services
   (or add Fledge there by hand). Choose its devices, in the order they
   show, and what it wakes to: the device chooser, the thermostat as the
   Nest always did, or one of the devices.

Change them any time from the integration's **Configure**. Removing the
integration puts the thermostat back as it was.

## What the thermostat may do

Only to the devices chosen here, and only: switch a light, switch or fan on
or off, set a light's brightness, set a thermostat's temperature, set a media player's
volume, and play or pause it.

It replaces the `nest_ui.yaml` blueprint: use one or the other.
