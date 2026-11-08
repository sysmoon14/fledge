#include <assert.h>
#include "../src/custom_flow.h"
static int last = -1, fail = 0;
static int request(int action) { last = action; return fail; }
static void matter_refresh_test(void) {
    struct custom_flow f = {.page = CUSTOM_MATTER};
    int started = 0;
    const char *paired = "Paired. Enable pairing to replace the controller.";
    assert(!custom_matter_refresh(&f, "Pairing open. Already paired; scan to replace the controller.", 0, &started, 0));
    assert(f.page == CUSTOM_MATTER && !started);
    /* Initial enrollment is already open without a settings request. */
    assert(custom_matter_refresh(&f, "Pairing open. Scan in your Matter app.", 0, &started, 0));
    assert(f.page == CUSTOM_MATTER_PAIRING && started);
    assert(custom_matter_refresh(&f, paired, 0, &started, 1));
    assert(f.page == CUSTOM_MATTER);
    f.page = CUSTOM_MATTER_PAIRING;
    assert(custom_back(&f) && f.page == CUSTOM_HOME);
    f.page = CUSTOM_MATTER_PAIRING; started = 0;
    /* A pre-existing Paired status cannot acknowledge a pending request. */
    assert(!custom_matter_refresh(&f, paired, 1, &started, 1));
    assert(!started && f.page == CUSTOM_MATTER_PAIRING);
    assert(!custom_matter_refresh(&f, "Removing controllers...", 0, &started, 2));
    assert(!started && f.page == CUSTOM_MATTER_PAIRING);
    /* Completion can occur entirely between polls, including re-pairing. */
    assert(custom_matter_refresh(&f, paired, 0, &started, 2));
    assert(started && f.page == CUSTOM_MATTER_PAIRING);
    assert(custom_matter_refresh(&f, paired, 0, &started, 3));
    assert(f.page == CUSTOM_MATTER && !f.action_failed);
    f.page = CUSTOM_MATTER_PAIRING; started = 0;
    assert(!custom_matter_refresh(&f, "Pairing open. Scan in your Matter app.", 1, &started, 1));
    assert(!started);
    assert(custom_matter_refresh(&f, "Pairing open. Scan in your Matter app.", 0, &started, 2));
    /* An acknowledged open window can outlive the request timeout. */
    assert(!custom_matter_refresh(&f, "Pairing open. Scan in your Matter app.", 0, &started, 120));
    assert(custom_matter_refresh(&f, paired, 0, &started, 121));
    assert(f.page == CUSTOM_MATTER);
    f.page = CUSTOM_MATTER_PAIRING; started = 0;
    assert(!custom_matter_refresh(&f, paired, 1, &started, 119));
    assert(custom_matter_refresh(&f, paired, 1, &started, 120));
    assert(f.page == CUSTOM_MATTER && f.action_failed);
    f.page = CUSTOM_MATTER_PAIRING; started = 1;
    assert(custom_matter_refresh(&f, "Matter not enabled", 0, &started, 3));
    assert(f.page == CUSTOM_MATTER && f.action_failed);
    f.page = CUSTOM_MATTER_PAIRING;
    assert(custom_matter_refresh(&f, "Ready to pair.", 0, &started, 3));
    assert(f.page == CUSTOM_MATTER && !f.action_failed);
    f.page = CUSTOM_ZIGBEE_PAIRING;
    assert(!custom_matter_refresh(&f, paired, 0, &started, 3));
    assert(f.page == CUSTOM_ZIGBEE_PAIRING);
}
static void streq(const char *a, const char *b) {
    while (*a && *a == *b) { a++; b++; }
    assert(*a == *b);
}
static void lights_test(void) {
    struct custom_lights lights, read;
    assert(!custom_lights_parse("", &lights));
    assert(!custom_lights_parse("7\n1 x 1 Snug\n", &lights));
    assert(custom_lights_parse("7\n", &lights) == 0 && lights.generation == 7);
    assert(custom_lights_parse("42\n1 60 1 Snug lamp\n0 -1 0 Hall\n", &lights) == 2);
    assert(lights.generation == 42 && lights.light[0].on && lights.light[0].brightness == 60);
    assert(lights.light[0].dimmable && !lights.light[1].on && lights.light[1].brightness == -1);
    streq(lights.light[0].name, "Snug lamp");
    streq(lights.light[1].name, "Hall");
    /* Names are truncated; out-of-range brightness is unknown; the list is capped. */
    assert(custom_lights_parse("1\n1 250 1 A very long light name that does not fit\n", &read) == 1);
    assert(read.light[0].brightness == -1 && read.light[0].name[CUSTOM_LIGHT_NAME - 1] == 0);
    assert(custom_lights_parse("1\n1 1 1 a\n1 1 1 b\n1 1 1 c\n1 1 1 d\n1 1 1 e\n1 1 1 f\n1 1 1 g\n", &read) == CUSTOM_LIGHTS_MAX);

    char text[64];
    custom_light_label(text, sizeof(text), &lights.light[0]); streq(text, "Snug lamp: 60%");
    custom_light_label(text, sizeof(text), &lights.light[1]); streq(text, "Hall: Off");
    lights.light[1].on = 1; custom_light_label(text, sizeof(text), &lights.light[1]); streq(text, "Hall: On");
    lights.light[1].on = 0;

    /* Home shows Lights first when configured; a choice opens the list. */
    assert(custom_home_count(8) == 2 && custom_home_action(0, 8) == 4 && custom_home_action(1, 8) == 2);
    assert(custom_home_action(0, 15) == 4 && custom_home_action(4, 15) == 2);
    struct custom_flow f = {0};
    struct custom_dial d = {0};
    assert(custom_choose(&f, 4, request) == 1 && f.page == CUSTOM_LIGHTS && f.light == 0);

    /* An on/off light toggles in place; a dimmable one opens its page. */
    assert(custom_lights_choose(&f, &lights, &d, 1) == 1 && f.page == CUSTOM_LIGHTS && lights.light[1].on);
    custom_light_request(text, sizeof(text), lights.generation, 1, &lights.light[1]); streq(text, "42 1 1 -1\n");
    assert(custom_lights_choose(&f, &lights, &d, 0) == 0 && f.page == CUSTOM_LIGHT && f.light == 0);

    /* The ring sets brightness and is sent once it settles. */
    assert(custom_light_turn(&lights.light[0], &d, 3) && lights.light[0].brightness == 62);
    assert(custom_light_turn(&lights.light[0], &d, -1) && lights.light[0].brightness == 60);
    assert(!custom_light_tick(&d));
    assert(custom_light_tick(&d) && !d.pending);
    custom_light_request(text, sizeof(text), lights.generation, 0, &lights.light[0]); streq(text, "42 0 1 60\n");
    /* Turning continuously still sends, every CUSTOM_LIGHT_LIVE ticks. */
    int sent = 0;
    for (int i = 0; i < CUSTOM_LIGHT_LIVE; i++) { custom_light_turn(&lights.light[0], &d, 1); sent += custom_light_tick(&d); }
    assert(sent == 1 && lights.light[0].brightness == 60 + 2 * CUSTOM_LIGHT_LIVE);
    /* Full scale holds at 100; down to 0 turns it off. */
    for (int i = 0; i < 40; i++) custom_light_turn(&lights.light[0], &d, 1);
    assert(lights.light[0].brightness == 100 && !custom_light_turn(&lights.light[0], &d, 1));
    for (int i = 0; i < 60; i++) custom_light_turn(&lights.light[0], &d, -1);
    assert(!lights.light[0].on && lights.light[0].brightness == 0);
    custom_light_request(text, sizeof(text), lights.generation, 0, &lights.light[0]); streq(text, "42 0 0 -1\n");
    /* A press back on asks for the light's last brightness. */
    assert(custom_lights_choose(&f, &lights, &d, 0) == 1 && lights.light[0].on && lights.light[0].brightness == -1);
    assert(!d.pending && d.hold == CUSTOM_LIGHT_HOLD);
    /* Turning up from off starts at the bottom. */
    custom_light_press(&lights.light[0], &d);
    custom_light_turn(&lights.light[0], &d, 1);
    assert(lights.light[0].on && lights.light[0].brightness == CUSTOM_LIGHT_STEP);

    /* Home Assistant's echo does not undo the held light; others update. */
    assert(custom_lights_parse("42\n0 -1 1 Snug lamp\n0 -1 0 Hall\n", &read) == 2);
    assert(custom_lights_merge(&lights, &read, 0) == 1);
    assert(lights.light[0].on && !lights.light[1].on);
    assert(custom_lights_merge(&lights, &read, -1) == 1 && !lights.light[0].on);
    assert(custom_lights_merge(&lights, &read, -1) == 0);
    assert(custom_lights_parse("43\n0 -1 1 Snug lamp\n", &read) == 1);
    assert(custom_lights_merge(&lights, &read, 0) == 2 && lights.count == 1);

    /* The brightness ring: its layout, and the ticks lit. */
    short xs[CUSTOM_RING_MAX], ys[CUSTOM_RING_MAX];
    assert(custom_ring_parse("3\n-10 20\n0 -230\n10 20\n", xs, ys) == 3);
    assert(xs[0] == -10 && ys[1] == -230 && xs[2] == 10);
    assert(!custom_ring_parse("", xs, ys) && !custom_ring_parse("3\n1 2\n", xs, ys));
    assert(!custom_ring_parse("99\n", xs, ys) && !custom_ring_parse("0\n", xs, ys));
    struct custom_light ring = {.on = 1, .brightness = 60, .dimmable = 1};
    assert(custom_light_ticks(&ring, 50) == 30);
    ring.brightness = 1; assert(custom_light_ticks(&ring, 50) == 1);
    ring.brightness = 100; assert(custom_light_ticks(&ring, 50) == 50);
    ring.brightness = -1; assert(custom_light_ticks(&ring, 50) == 50);
    ring.on = 0; assert(custom_light_ticks(&ring, 50) == 0);
    /* Idle ticks count up; Back returns to the list, then home. */
    d.idle = 0;
    for (int i = 0; i < CUSTOM_LIGHT_IDLE; i++) custom_light_tick(&d);
    assert(d.idle == CUSTOM_LIGHT_IDLE);
    assert(custom_back(&f) && f.page == CUSTOM_LIGHTS && f.light == 0);
    assert(custom_lights_choose(&f, &lights, &d, lights.count) == 0 && f.page == CUSTOM_HOME);
    f.page = CUSTOM_LIGHTS;
    assert(custom_back(&f) && f.page == CUSTOM_HOME);
    /* Opened with lights, the list is the top page: More, Done, and Back leaves. */
    custom_open(&f, 2);
    assert(f.page == CUSTOM_LIGHTS && f.lights_first && f.light == 0);
    assert(custom_lights_rows(2) == 4 && custom_lights_rows(CUSTOM_LIGHTS_MAX) == CUSTOM_LIGHTS_MAX + 1);
    assert(custom_lights_rows(CUSTOM_LIGHTS_MAX - 1) == CUSTOM_LIGHT_ROWS_MAX);
    assert(!custom_back(&f) && f.page == CUSTOM_LIGHTS);
    assert(custom_lights_choose(&f, &lights, &d, lights.count + 1) == 2);
    assert(custom_lights_choose(&f, &lights, &d, lights.count + 2) == 0 && f.page == CUSTOM_LIGHTS);
    assert(custom_lights_choose(&f, &lights, &d, lights.count) == 0 && f.page == CUSTOM_HOME);
    assert(custom_choose(&f, 4, request) == 1 && f.page == CUSTOM_LIGHTS);
    assert(!custom_back(&f));
    custom_open(&f, 0);
    assert(f.page == CUSTOM_HOME && !f.lights_first);
    /* With a full list, More takes the last slot and Done is on the UNFR0ZEN page. */
    struct custom_lights full;
    assert(custom_lights_parse("5\n1 1 1 a\n1 1 1 b\n1 1 1 c\n1 1 1 d\n1 1 1 e\n1 1 1 f\n", &full) == CUSTOM_LIGHTS_MAX);
    custom_open(&f, full.count);
    assert(custom_lights_choose(&f, &full, &d, CUSTOM_LIGHTS_MAX + 1) == 0 && f.page == CUSTOM_LIGHTS);
    assert(custom_lights_choose(&f, &full, &d, CUSTOM_LIGHTS_MAX) == 0 && f.page == CUSTOM_HOME);
    f.page = CUSTOM_LIGHTS; f.lights_first = 0;
    /* A list shrunk under the light page sends nothing and leaves it. */
    f.page = CUSTOM_LIGHT; f.light = 3;
    assert(custom_lights_choose(&f, &lights, &d, 0) == 0 && f.page == CUSTOM_LIGHTS);
}
int main(void) {
    matter_refresh_test();
    lights_test();
    const int actions[8][4] = {{2, -1, -1, -1}, {0, 2, -1, -1}, {1, 2, -1, -1}, {0, 1, 2, -1},
                               {3, 2, -1, -1}, {0, 3, 2, -1}, {1, 3, 2, -1}, {0, 1, 3, 2}};
    for (unsigned int mask = 0; mask < 8; mask++) {
        for (int i = 0; i < 4; i++) assert(custom_home_action(i, mask) == actions[mask][i]);
        assert(custom_home_action(custom_home_count(mask) - 1, mask) == 2);
        assert(custom_home_action(-1, mask) == -1);
    }
    struct custom_flow f = {0};
    assert(custom_choose(&f, 0, request) == 1 && f.page == CUSTOM_MATTER);
    last = -1;
    assert(custom_choose(&f, 0, request) == 1 && last == -1 && f.page == CUSTOM_MATTER_CONFIRM);
    assert(custom_choose(&f, 1, request) == 1 && last == -1 && f.page == CUSTOM_MATTER);
    custom_choose(&f, 0, request);
    assert(custom_choose(&f, 0, request) == 1 && last == 0 && f.page == CUSTOM_MATTER_PAIRING);
    assert(custom_choose(&f, 0, request) == 1 && f.page == CUSTOM_HOME);
    custom_choose(&f, 0, request);
    custom_choose(&f, 0, request);
    fail = 1; custom_choose(&f, 0, request); assert(f.action_failed && f.page == CUSTOM_MATTER);
    assert(custom_back(&f) && f.page == CUSTOM_HOME && !f.action_failed);
    custom_choose(&f, 1, request); assert(f.page == CUSTOM_ZIGBEE);
    last = -1; custom_choose(&f, 0, request);
    assert(f.page == CUSTOM_ZIGBEE_CONFIRM && last == -1);
    custom_choose(&f, 1, request); assert(f.page == CUSTOM_ZIGBEE && last == -1);
    custom_choose(&f, 0, request); fail = 0; custom_choose(&f, 0, request);
    assert(f.page == CUSTOM_ZIGBEE_PAIRING && last == 1 && !f.action_failed);
    assert(custom_back(&f) && f.page == CUSTOM_ZIGBEE);
    custom_back(&f); assert(f.page == CUSTOM_HOME && f.parent_selection == 1);
    assert(!custom_back(&f)); assert(custom_choose(&f, 2, request) == 2);
    last = -1; assert(custom_choose(&f, 3, request) == 1 && last == 2 && f.page == CUSTOM_HOME && !f.action_failed);
    fail = 1; custom_choose(&f, 3, request); assert(f.action_failed && f.page == CUSTOM_HOME);
    return 0;
}
