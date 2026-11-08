# Fledge for Home Assistant

Home Assistant on the screen of a Nest Learning Thermostat (3rd generation)
running [Fledge](https://github.com/sysmoon14/fledge). Turning the ring moves
from the thermostat to a card for each room's lights, each other thermostat
and each media player; pressing controls the card.

## Install

1. The thermostat runs Fledge and is connected to Home Assistant over MQTT
   (it shows up as a "Nest Thermostat" device).
2. In HACS, add `https://github.com/sysmoon14/fledge` as a custom repository
   of type Integration, and download Fledge. Restart Home Assistant.
3. Home Assistant offers the thermostat under Settings → Devices & services
   (or add Fledge there by hand). Choose its rooms, thermostats and media
   players.

Change the cards any time from the integration's **Configure**. Removing the
integration puts the thermostat back as it was.

## What the thermostat may do

Only to the entities chosen here, and only: switch a light on or off, set a
light's brightness, set a thermostat's temperature, set a media player's
volume, and play or pause it.

It replaces the `nest_ui.yaml` blueprint: use one or the other.
