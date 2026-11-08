"""Constants for Fledge, the companion of a Nest thermostat running Fledge."""

DOMAIN = "fledge"

# nestlocal, on the thermostat, publishes and listens under nestlocal/<serial>.
TOPIC_PREFIX = "nestlocal"

CONF_SERIAL = "serial"
# The devices the thermostat shows, a card each, in order.
CONF_ENTITIES = "entities"
# What it wakes to: the chooser or itself, unless CONF_WAKE_DEVICE names a card.
CONF_WAKE = "wake"
CONF_WAKE_DEVICE = "wake_device"
# Before 0.4.4: rooms' lights, less some, then thermostats and media players.
CONF_AREAS = "areas"
CONF_EXCLUDE = "exclude"
CONF_CLIMATES = "climates"
CONF_MEDIA = "media"

WAKE_CHOOSER = "chooser"
WAKE_THERMOSTAT = "thermostat"

# What can be a card: switched (and dimmed, a light), set, or turned up.
SWITCHED = ("light", "switch", "fan", "input_boolean")
DOMAINS = [*SWITCHED, "climate", "media_player"]

# The cards the thermostat shows (retained) and what it asks for.
CARDS_TOPIC = "{prefix}/{serial}/set/ui"
COMMAND_TOPIC = "{prefix}/{serial}/ui/command"

# nestlocal's device in Home Assistant: identifiers ("mqtt", "nestlocal_<serial>").
DEVICE_IDENTIFIER = "nestlocal_{serial}"
