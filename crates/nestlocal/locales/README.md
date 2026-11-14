# Screen message catalogs

`en.json` is the source catalog for the text nestlocal draws on the
thermostat's screen: its cards, dials, menus, glance faces and alarm screens.
Message IDs are stable, context-specific keys; values contain the displayed
English text. Every other file holds one language, named by its code
(`nl.json`), with the same message IDs. A message missing from a catalog is
shown in English.

Code uses `crate::tr!("message.id")` for plain text and
`crate::tr!("message.id", name = value)` for a message containing `{name}`.
Arguments are substituted by name, so translations can reorder them. A word
Home Assistant sends as a value (a climate mode, weather condition, cover
state or alarm mode) is looked up with `locale::named("climate", value)` as
`climate.heat_cool`, and shown as Home Assistant names it when no catalog has
it; day and month names use `locale::numbered("day", i)`, Sunday and January
being 0. The tests check every key and prefix these calls name.

## Adding languages

A language is one JSON file beside `en.json` and one line in `src/locale.rs`.

1. **Copy `en.json`** to a file named with the language's code, `de.json` for
   German. A catalog answers the language part of a locale, so `de` serves
   `de_DE` and `de_AT` alike.
2. **Translate the values.** Message IDs stay exactly as they are, every
   placeholder keeps its English name (`{count}`, `{device}`), and a `\n` in a
   message starts its second line. Placeholders are substituted by name, so a
   translation can put them in its own order; keep sentences together rather
   than concatenating translated fragments. Translate every message, since the
   test below asks for a catalog as complete as English's — leave a message in
   English only while you work through the file.
3. **Add the file to `SOURCES`** in `src/locale.rs`, which pairs each language
   code with the JSON embedded from it. The count in `SOURCES`'s type is the
   number of catalogs, so it grows with the new one.
4. **Check the catalog.** `cargo test -p nestlocal --locked locale` fails on a
   missing, extra or empty message, and on a placeholder that no longer
   matches English's. The same test fails when `en.json` gains a message that
   no code draws, or code draws one `en.json` lacks.
5. **Read it on the screen.** The screen is small and round: a line that runs
   out of room is shortened with `…`, and some lines low on the face reach the
   ring before that, so check the long messages there. The `ui_shots` test in
   `src/ui/mod.rs` draws every screen to PNGs, in `UNFR0ZEN_UI_LANGUAGE`'s
   language when it is set and with the unit's fonts:

   ```sh
   UNFR0ZEN_UI_LANGUAGE=de UNFR0ZEN_UI_SHOTS=/tmp/shots \
   UNFR0ZEN_UI_FONTS=/nestlabs/share/fonts \
   cargo test -p nestlocal --locked ui_shots -- --ignored
   ```

Do not translate message IDs, placeholders, icon names, or values exchanged
with Home Assistant. Names from Home Assistant (rooms, lights, scenes, a
select's options, a face's lines) are shown as Home Assistant sends them, in
the language they were written in. The glance's day and month names come from
Home Assistant when it sends them, otherwise from the catalog's `day.*` and
`month.*`. Numbers keep a decimal point, and a 12-hour clock its AM and PM,
in every language: they are drawn by code shared with English rather than
taken from a catalog.

## The screen's language

nestlocal chooses the language once, when it starts: `-language` if given
(`nl` or `nl_NL`), otherwise the language the Nest is set to, which nlclient
keeps in `/media/user-config/settings.config` as
`<s key="current_locale" value="nl_NL"/>`. English is used when neither names
a language with a catalog. A language changed in the Nest's settings takes
effect the next time nestlocal starts.

nestlocal logs its choice as `nestlocal: the screen's language is nl (...)`,
with where it came from, or why the Nest's setting was not used. On a gen 3
with SSH (`--ssh`), these show the Nest's setting and nestlocal's choice;
`monit restart nestlocal` logs the choice again:

```sh
ssh root@<unit> grep current_locale /media/user-config/settings.config
ssh root@<unit> grep -h "\"screen's language\"" /media/log/messages
```

The page Fledge adds to the Nest's settings (`platform/common/nlclient`) is
not part of these catalogs and remains in English.
