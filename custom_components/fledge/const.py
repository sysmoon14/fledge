"""Constants for Fledge, the companion of a Nest thermostat running Fledge."""

DOMAIN = "fledge"

# nestlocal, on the thermostat, publishes and listens under nestlocal/<serial>.
TOPIC_PREFIX = "nestlocal"

CONF_SERIAL = "serial"
CONF_AREAS = "areas"
CONF_CLIMATES = "climates"
CONF_MEDIA = "media"

# The cards the thermostat shows (retained) and what it asks for.
CARDS_TOPIC = "{prefix}/{serial}/set/ui"
COMMAND_TOPIC = "{prefix}/{serial}/ui/command"

# nestlocal's device in Home Assistant: identifiers ("mqtt", "nestlocal_<serial>").
DEVICE_IDENTIFIER = "nestlocal_{serial}"

# The thermostat lists at most this many lights on a card.
MAX_LIGHTS = 8
