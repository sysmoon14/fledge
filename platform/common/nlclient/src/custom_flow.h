#ifndef UNFR0ZEN_FLOW_H
#define UNFR0ZEN_FLOW_H
/* Firmware-independent navigation. Actions are acknowledged by nestlocal. */
enum custom_page {
    CUSTOM_HOME, CUSTOM_MATTER, CUSTOM_ZIGBEE, CUSTOM_ZIGBEE_CONFIRM,
    CUSTOM_MATTER_PAIRING, CUSTOM_ZIGBEE_PAIRING, CUSTOM_MATTER_CONFIRM,
    CUSTOM_LIGHTS, CUSTOM_LIGHT
};
/* light: the light chosen on the Lights page, and that page's selection.
 * lights_first: the screen opened on Lights (Home Assistant has some), which
 * is then its top page: More leads to the UNFR0ZEN page, Back leaves. */
struct custom_flow { enum custom_page page; int parent_selection; int action_failed; int light; int lights_first; };
static int custom_activity(enum custom_page page) {
    return page == CUSTOM_MATTER_PAIRING || page == CUSTOM_ZIGBEE_PAIRING;
}
static inline int custom_matter(enum custom_page page) {
    return page == CUSTOM_MATTER || page == CUSTOM_MATTER_PAIRING || page == CUSTOM_MATTER_CONFIRM;
}
/* Canonical home actions: Matter=0, Zigbee=1, Done=2, Doom=3, Lights=4,
 * shown as Lights, Matter, Zigbee, Doom, Done. Lights (feature 8) is there
 * while Home Assistant has configured any. Compact the visible list without
 * ever interpreting Zigbee-only index 0 as Matter. */
static inline int custom_home_count(unsigned int features) {
    return 1 + !!(features & 1) + !!(features & 2) + !!(features & 4) + !!(features & 8);
}
static inline int custom_home_action(int choice, unsigned int features) {
    int index = 0;
    if ((features & 8) && choice == index++) return 4;
    if ((features & 1) && choice == index++) return 0;
    if ((features & 2) && choice == index++) return 1;
    if ((features & 4) && choice == index++) return 3;
    return choice == index ? 2 : -1;
}
/* The page the screen opens on: Lights while Home Assistant has configured
 * any, for the fewest presses from the thermostat's menu; else UNFR0ZEN's. */
static inline void custom_open(struct custom_flow *flow, int light_count) {
    flow->page = light_count > 0 ? CUSTOM_LIGHTS : CUSTOM_HOME;
    flow->lights_first = light_count > 0;
    flow->parent_selection = 0;
    flow->action_failed = 0;
    flow->light = 0;
}
static int custom_back(struct custom_flow *flow) {
    if (flow->page == CUSTOM_HOME) return 0;
    if (flow->page == CUSTOM_LIGHTS && flow->lights_first) return 0;
    flow->page = flow->page == CUSTOM_LIGHT ? CUSTOM_LIGHTS :
                 flow->page == CUSTOM_MATTER_CONFIRM ? CUSTOM_MATTER :
                 flow->page == CUSTOM_ZIGBEE_PAIRING || flow->page == CUSTOM_ZIGBEE_CONFIRM ? CUSTOM_ZIGBEE : CUSTOM_HOME;
    flow->action_failed = 0;
    return 1;
}
static inline int starts_with(const char *text, const char *prefix) {
    while (*prefix) { if (*text++ != *prefix++) return 0; }
    return 1;
}
/* Request consumption acknowledges an attempt even when polling misses its
 * open window. Wait another poll before accepting a terminal status, since
 * the worker removes the request before publishing its new status. */
static inline int custom_matter_refresh(struct custom_flow *flow, const char *status,
                                       int pending, int *started, unsigned int ticks) {
    if (flow->page == CUSTOM_MATTER && starts_with(status, "Pairing open") &&
        !starts_with(status, "Pairing open. Already paired")) {
        flow->page = CUSTOM_MATTER_PAIRING;
        *started = 1;
        return 1;
    }
    if (flow->page != CUSTOM_MATTER_PAIRING) return 0;
    if (starts_with(status, "Matter not enabled") || starts_with(status, "Starting Matter") ||
        starts_with(status, "Not enabled") || starts_with(status, "Unavailable") ||
        (!*started && pending && ticks >= 120)) {
        flow->page = CUSTOM_MATTER;
        flow->action_failed = 1;
        return 1;
    }
    if (starts_with(status, "Removing controllers")) return 0;
    if (!*started && !pending) {
        *started = 1;
        return 1;
    }
    if (*started && !pending && (starts_with(status, "Paired") || starts_with(status, "Ready to pair") ||
                                starts_with(status, "Pairing timed out"))) {
        flow->page = CUSTOM_MATTER;
        flow->action_failed = 0;
        return 1;
    }
    return 0;
}
/* Requests: Matter's pairing=0, Zigbee's=1, Doom's launch=2. */
static int custom_choose(struct custom_flow *flow, int choice, int (*request)(int)) {
    if (choice < 0 || choice > 4) return 1;
    flow->action_failed = 0;
    if (flow->page == CUSTOM_HOME) {
        if (choice == 2) return 2;
        if (choice == 4) {
            flow->page = CUSTOM_LIGHTS;
            flow->light = 0;
            return 1;
        }
        /* nestlocal takes the screen over for Doom and gives it back after. */
        if (choice == 3) {
            flow->action_failed = request(2) != 0;
            return 1;
        }
        flow->parent_selection = choice;
        flow->page = choice == 0 ? CUSTOM_MATTER : CUSTOM_ZIGBEE;
    } else if (custom_activity(flow->page)) {
        if (choice == 0) custom_back(flow);
    } else if (flow->page == CUSTOM_MATTER_CONFIRM) {
        if (choice == 0) {
            flow->action_failed = request(0) != 0;
            flow->page = flow->action_failed ? CUSTOM_MATTER : CUSTOM_MATTER_PAIRING;
        } else if (choice == 1) custom_back(flow);
    } else if (flow->page == CUSTOM_ZIGBEE_CONFIRM) {
        if (choice == 0) {
            flow->action_failed = request(1) != 0;
            flow->page = flow->action_failed ? CUSTOM_ZIGBEE : CUSTOM_ZIGBEE_PAIRING;
        } else if (choice == 1) custom_back(flow);
    } else if (choice == 0) {
        if (flow->page == CUSTOM_MATTER) {
            flow->page = CUSTOM_MATTER_CONFIRM;
        } else flow->page = CUSTOM_ZIGBEE_CONFIRM;
    } else if (choice == 1) custom_back(flow);
    return 1;
}
/* Home Assistant's lights, as nestlocal publishes them in /var/run/unfr0zen/lights:
 *   <generation>\n
 *   <on 0|1> <brightness -1..100> <dimmable 0|1> <name>\n   (one per light)
 * The generation identifies the list (its entities, names and order): a
 * request carries it, so one made against an older list is dropped. A
 * brightness of -1 is unknown, or in a request, the light's last. */
#define CUSTOM_LIGHTS_MAX 6
#define CUSTOM_LIGHT_NAME 32
/* Percent per ring event, and the 250 ms ticks the light page waits. */
#define CUSTOM_LIGHT_STEP 2
#define CUSTOM_LIGHT_QUIET 2   /* the ring still, before brightness is sent */
#define CUSTOM_LIGHT_LIVE 4    /* while turning, brightness is sent this often */
#define CUSTOM_LIGHT_HOLD 8    /* our change wins over Home Assistant's echo */
#define CUSTOM_LIGHT_IDLE 20   /* no input: back to the Lights page */
struct custom_light { int on; int brightness; int dimmable; char name[CUSTOM_LIGHT_NAME]; };
struct custom_lights { unsigned int generation; int count; struct custom_light light[CUSTOM_LIGHTS_MAX]; };
/* The light page's ring and press: a change waits for the ring to settle. */
struct custom_dial { int pending; int quiet; int held; int hold; int idle; };
/* The screen links without libc: copy bytes rather than assign structs,
 * which the compiler would turn into calls to memcpy. */
static inline void custom_copy(void *to, const void *from, unsigned int count) {
    unsigned char *dest = to;
    const unsigned char *source = from;
    while (count--) *dest++ = *source++;
}
static inline const char *custom_number(const char *text, int *out) {
    int sign = 1, value = 0, digits = 0;
    while (*text == ' ') text++;
    if (*text == '-') { sign = -1; text++; }
    while (*text >= '0' && *text <= '9' && digits < 10) { value = value * 10 + (*text++ - '0'); digits++; }
    if (!digits) return 0;
    *out = sign * value;
    return text;
}
/* The light count, or 0 for a missing or malformed list. */
static inline int custom_lights_parse(const char *text, struct custom_lights *out) {
    int generation = 0;
    out->count = 0;
    out->generation = 0;
    if (!(text = custom_number(text, &generation)) || *text != '\n') return 0;
    out->generation = (unsigned int)generation;
    text++;
    while (*text && out->count < CUSTOM_LIGHTS_MAX) {
        struct custom_light *light = &out->light[out->count];
        int on, brightness, dimmable, n = 0;
        if (!(text = custom_number(text, &on)) || !(text = custom_number(text, &brightness)) ||
            !(text = custom_number(text, &dimmable)) || *text != ' ') { out->count = 0; return 0; }
        text++;
        while (*text && *text != '\n') {
            if (n < CUSTOM_LIGHT_NAME - 1) light->name[n++] = *text;
            text++;
        }
        light->name[n] = 0;
        if (*text == '\n') text++;
        light->on = !!on;
        light->brightness = brightness < -1 || brightness > 100 ? -1 : brightness;
        light->dimmable = !!dimmable;
        out->count++;
    }
    return out->count;
}
static inline int custom_light_same(const struct custom_light *a, const struct custom_light *b) {
    if (a->on != b->on || a->brightness != b->brightness || a->dimmable != b->dimmable) return 0;
    for (int i = 0; i < CUSTOM_LIGHT_NAME; i++) {
        if (a->name[i] != b->name[i]) return 0;
        if (!a->name[i]) break;
    }
    return 1;
}
/* Takes a newly read list. 2: a different list; 1: a light's state changed;
 * 0: nothing to redraw. The held light keeps the state we just set, until
 * Home Assistant has had time to report it. */
static inline int custom_lights_merge(struct custom_lights *current, const struct custom_lights *read, int held) {
    if (read->generation != current->generation || read->count != current->count) {
        custom_copy(current, read, sizeof(*current));
        return 2;
    }
    int changed = 0;
    for (int i = 0; i < read->count; i++) {
        if (i == held) continue;
        if (!custom_light_same(&current->light[i], &read->light[i])) {
            custom_copy(&current->light[i], &read->light[i], sizeof(read->light[i]));
            changed = 1;
        }
    }
    return changed;
}
/* A ring event on the light page: 1 when the brightness changed. Turning
 * up from off starts at the bottom; turning down to 0 turns it off. */
static inline int custom_light_turn(struct custom_light *light, struct custom_dial *dial, int value) {
    dial->idle = 0;
    if (!light->dimmable || !value) return 0;
    int brightness = light->on ? (light->brightness < 0 ? 100 : light->brightness) : 0;
    brightness += value > 0 ? CUSTOM_LIGHT_STEP : -CUSTOM_LIGHT_STEP;
    if (brightness < 0) brightness = 0;
    if (brightness > 100) brightness = 100;
    if (brightness == light->brightness && light->on == (brightness > 0)) return 0;
    light->on = brightness > 0;
    light->brightness = brightness;
    if (!dial->pending) dial->held = 0;
    dial->pending = 1;
    dial->quiet = 0;
    dial->hold = CUSTOM_LIGHT_HOLD;
    return 1;
}
/* A press toggles; the change is sent at once. Back on from 0 asks Home
 * Assistant for the light's last brightness. */
static inline void custom_light_press(struct custom_light *light, struct custom_dial *dial) {
    light->on = !light->on;
    if (light->on && light->brightness == 0) light->brightness = -1;
    dial->pending = 0;
    dial->idle = 0;
    dial->hold = CUSTOM_LIGHT_HOLD;
}
/* Each 250 ms tick of the light page: 1 when the pending brightness is to
 * be sent now, once the ring has settled or while it keeps turning. */
static inline int custom_light_tick(struct custom_dial *dial) {
    dial->idle++;
    if (dial->hold > 0) dial->hold--;
    if (!dial->pending) return 0;
    dial->quiet++;
    dial->held++;
    if (dial->quiet < CUSTOM_LIGHT_QUIET && dial->held < CUSTOM_LIGHT_LIVE) return 0;
    dial->pending = 0;
    dial->held = 0;
    return 1;
}
/* The Lights page's rows: the lights, then More (the UNFR0ZEN page) when it
 * is the top page, else Back; then Done while a choice slot is left. */
#define CUSTOM_LIGHT_ROWS_MAX (CUSTOM_LIGHTS_MAX + 1)
static inline int custom_lights_done(int count) { return count + 1 < CUSTOM_LIGHT_ROWS_MAX; }
static inline int custom_lights_rows(int count) { return count + 1 + custom_lights_done(count); }
/* A choice on the Lights page. 1 when the chosen light is to be sent (an
 * on/off light toggles in place); a dimmable one opens its own page, where
 * the ring sets brightness and a press toggles. 2: Done, leave the screen. */
static inline int custom_lights_choose(struct custom_flow *flow, struct custom_lights *lights,
                                       struct custom_dial *dial, int choice) {
    if (flow->page == CUSTOM_LIGHT) {
        if (flow->light >= lights->count) { custom_back(flow); return 0; }
        custom_light_press(&lights->light[flow->light], dial);
        return 1;
    }
    if (choice < 0 || choice >= custom_lights_rows(lights->count)) return 0;
    if (choice == lights->count + 1) return 2;
    if (choice == lights->count) {
        flow->page = CUSTOM_HOME;
        flow->action_failed = 0;
        return 0;
    }
    flow->light = choice;
    if (lights->light[choice].dimmable) {
        flow->page = CUSTOM_LIGHT;
        dial->pending = dial->quiet = dial->held = dial->hold = dial->idle = 0;
        return 0;
    }
    custom_light_press(&lights->light[choice], dial);
    return 1;
}
static inline unsigned int custom_append(char *out, unsigned int at, unsigned int size, const char *text) {
    while (*text && at + 1 < size) out[at++] = *text++;
    out[at] = 0;
    return at;
}
static inline unsigned int custom_append_number(char *out, unsigned int at, unsigned int size, int value) {
    char digits[12];
    int n = 0;
    unsigned int magnitude = value < 0 ? (unsigned int)-value : (unsigned int)value;
    if (value < 0) at = custom_append(out, at, size, "-");
    do { digits[n++] = (char)('0' + magnitude % 10); magnitude /= 10; } while (magnitude);
    while (n && at + 1 < size) out[at++] = digits[--n];
    out[at] = 0;
    return at;
}
/* What the light is: "60%", "On" or "Off". */
static inline unsigned int custom_light_value(char *out, unsigned int at, unsigned int size, const struct custom_light *light) {
    if (!light->on) return custom_append(out, at, size, "Off");
    if (!light->dimmable || light->brightness < 0) return custom_append(out, at, size, "On");
    at = custom_append_number(out, at, size, light->brightness);
    return custom_append(out, at, size, "%");
}
/* The light page's brightness ring, from /var/run/unfr0zen/light-ring.layout:
 * "<count>\n" then "<x> <y>\n" per tick, its image's top left relative to the
 * display's centre. The count, or 0 for a missing or malformed layout. */
#define CUSTOM_RING_MAX 64
static inline int custom_ring_parse(const char *text, short *xs, short *ys) {
    int count = 0;
    if (!(text = custom_number(text, &count)) || count < 1 || count > CUSTOM_RING_MAX) return 0;
    for (int i = 0; i < count; i++) {
        int x, y;
        while (*text == '\n' || *text == '\r') text++;
        if (!(text = custom_number(text, &x)) || !(text = custom_number(text, &y))) return 0;
        xs[i] = (short)x;
        ys[i] = (short)y;
    }
    return count;
}
/* Ticks lit for the light: none when off, all at an unknown brightness, and
 * at least one for any brightness above 0. */
static inline int custom_light_ticks(const struct custom_light *light, int ticks) {
    if (!light->on) return 0;
    if (light->brightness < 0) return ticks;
    int lit = (light->brightness * ticks + 50) / 100;
    return lit < 1 ? 1 : lit > ticks ? ticks : lit;
}
/* Its row on the Lights page: "Snug lamp: 60%". */
static inline void custom_light_label(char *out, unsigned int size, const struct custom_light *light) {
    unsigned int at = custom_append(out, 0, size, light->name);
    at = custom_append(out, at, size, ": ");
    custom_light_value(out, at, size, light);
}
/* The request for nestlocal: "<generation> <index> <on> <brightness>\n". */
static inline void custom_light_request(char *out, unsigned int size, unsigned int generation,
                                        int index, const struct custom_light *light) {
    unsigned int at = 0;
    char number[12];
    int n = 0;
    do { number[n++] = (char)('0' + generation % 10); generation /= 10; } while (generation);
    while (n && at + 1 < size) out[at++] = number[--n];
    out[at] = 0;
    at = custom_append(out, at, size, " ");
    at = custom_append_number(out, at, size, index);
    at = custom_append(out, at, size, light->on ? " 1 " : " 0 ");
    at = custom_append_number(out, at, size, light->on ? light->brightness : -1);
    custom_append(out, at, size, "\n");
}
#endif
