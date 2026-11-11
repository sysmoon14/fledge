//! The patch page's form: the options the run offers, as the person types
//! them, and the [`Config`] they build.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use eframe::egui;
use fledge_host::diamond3::usb::EntryPoint;
use fledge_host::flow::default_data_dir;
use fledge_host::{Config, Device, PatchInputs, Slot, DEFAULT_MATTER_CODE, DEFAULT_NTP_SERVER};

use crate::bus::Bus;
use crate::{spawn_dialog, WAIT_USB};

/// The patch options the form offers, as strings the user types.
#[derive(Default)]
pub struct Form {
    backplate_sim: bool,
    block_cloud: bool,
    ntp: Toggle,
    pub mqtt: Toggle,
    mqtt_user: String,
    mqtt_password: String,
    mqtt_tls: bool,
    mqtt_ca: FilePick,
    /// Install Fledge's releases from Home Assistant's update entity.
    pub updates: bool,
    /// Back up the whole NAND before patching (the review step's choice).
    pub backup_first: bool,
    pub matter: MatterCode,
    matter_thread: bool,
    zigbee: bool,
    wwn_boiler: bool,
    pub entry_point: EntryPoint,
    pub ssh: bool,
    usb_shell: bool,
    root_password: String,
    // The advanced section: the CLI's options a run rarely needs.
    dry_run: bool,
    /// Save the root volume before patching.
    backup_volume: bool,
    slot: Option<Slot>,
    /// The device to expect instead of the one USB names.
    device: Option<Device>,
    /// The device last known, kept while no unit is on the bus.
    last_device: Option<Device>,
    pub doom: bool,
    external_temperature: bool,
    overlay: FilePick,
    cloud_url: Toggle,
    ssh_key: FilePick,
    /// SSH also takes root's password; off, keys only.
    pub ssh_password_login: bool,
    /// Why the last Create a key for me failed.
    ssh_key_error: Option<String>,
}

/// A file chosen with the platform's picker: the path, and the dialog a
/// click opened with the channel its choice arrives on over the following
/// frames.
#[derive(Default)]
pub struct FilePick {
    path: Option<PathBuf>,
    pending: Option<Receiver<Option<PathBuf>>>,
}

impl FilePick {
    /// Takes the dialog's answer when it arrives.
    fn drain(&mut self) {
        if let Some(receiver) = self.pending.take() {
            match receiver.try_recv() {
                Ok(Some(path)) => self.path = Some(path),
                Ok(None) => {}
                Err(_) => self.pending = Some(receiver),
            }
        }
    }

    /// The button that opens the picker for `kind` files, then the chosen
    /// file's name with a clear button.
    fn ui(&mut self, ui: &mut egui::Ui, label: impl Into<egui::WidgetText>, kind: &str, extensions: &[&str]) {
        self.ui_sized(ui, label, kind, extensions, 0.0);
    }

    /// The button at least `width` wide; the chosen file's name beside it,
    /// cut to the room left, with a clear button.
    fn ui_sized(&mut self, ui: &mut egui::Ui, label: impl Into<egui::WidgetText>, kind: &str, extensions: &[&str], width: f32) {
        if width == 0.0 {
            self.button(ui, label, kind, extensions, 0.0);
            self.chosen(ui);
            return;
        }
        // In a column: the button, then the chosen file under it, within
        // the button's width.
        ui.vertical(|ui| {
            ui.set_max_width(width);
            self.button(ui, label, kind, extensions, width);
            if self.path.is_some() {
                ui.horizontal(|ui| self.chosen(ui));
            }
        });
    }

    fn button(&mut self, ui: &mut egui::Ui, label: impl Into<egui::WidgetText>, kind: &str, extensions: &[&str], width: f32) {
        if ui.add(egui::Button::new(label).min_size(egui::vec2(width, 0.0)))
            .on_hover_text(crate::tr!("patch.file.choose_hint", kind = kind))
            .clicked() && self.pending.is_none() {
            let (sender, receiver) = std::sync::mpsc::channel();
            let dialog = rfd::AsyncFileDialog::new().add_filter(kind, extensions).pick_file();
            spawn_dialog(Box::new(dialog), sender);
            self.pending = Some(receiver);
        }
    }

    /// The chosen file's name, cut to the room left, with a clear button.
    fn chosen(&mut self, ui: &mut egui::Ui) {
        if let Some(chosen) = self.path.clone() {
            if ui.small_button(crate::tr!("patch.file.clear")).on_hover_text(crate::tr!("patch.file.clear_hint")).clicked() {
                self.path = None;
            }
            let name = chosen.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            ui.add(egui::Label::new(egui::RichText::new(name).weak().size(12.0)).truncate())
                .on_hover_text(chosen.display().to_string());
        }
    }
}

/// Matter, with the pairing code to enter on the phone (prefilled).
pub struct MatterCode {
    pub on: bool,
    pub code: String,
}

impl Default for MatterCode {
    fn default() -> Self {
        MatterCode { on: false, code: DEFAULT_MATTER_CODE.into() }
    }
}

/// An option that is off, or on with a value.
#[derive(Default)]
pub struct Toggle {
    pub on: bool,
    pub value: String,
}

impl Form {
    /// The flow's starting choices: Home Assistant with updates, Google
    /// blocked with time from the default server, and a backup first.
    pub fn recommended() -> Self {
        let mut form = Form { block_cloud: true, updates: true, backup_first: true, ..Form::default() };
        form.mqtt.on = true;
        form.ntp = Toggle { on: true, value: DEFAULT_NTP_SERVER.into() };
        form
    }

    /// Whether nestlocal takes the radio from a Heat Link.
    fn holds_radio(&self) -> bool {
        self.zigbee || (self.matter.on && self.matter_thread)
    }

    /// Whether the options install nestlocal.
    fn nestlocal(&self) -> bool {
        self.mqtt.on || self.matter.on || self.zigbee || self.doom
    }

    pub fn inputs(&self) -> PatchInputs {
        let text = |s: &str| (!s.is_empty()).then(|| s.to_string());
        PatchInputs {
            root_password: text(&self.root_password),
            ntp_server: self.ntp.on.then(|| self.ntp.value.clone()),
            // nestlocal takes nlclient's cloud and builds its own overlay.
            cloud_url: (self.cloud_url.on && !self.nestlocal()).then(|| self.cloud_url.value.clone()),
            backplate_sim: self.backplate_sim,
            block_cloud: self.block_cloud,
            overlay: if self.nestlocal() { None } else { self.overlay.path.clone() },
            mqtt: self.mqtt.on.then(|| self.mqtt.value.clone()),
            mqtt_user: self.mqtt.on.then(|| text(&self.mqtt_user)).flatten(),
            mqtt_password: self.mqtt.on.then(|| text(&self.mqtt_password)).flatten(),
            mqtt_tls: self.mqtt.on && self.mqtt_tls,
            // The CA field only shows under TLS, so it only sends under TLS.
            mqtt_ca: (self.mqtt.on && self.mqtt_tls).then(|| self.mqtt_ca.path.clone()).flatten(),
            updates: self.mqtt.on && self.updates,
            matter: self.matter.on,
            matter_thread: self.matter.on && self.matter_thread,
            matter_code: self.matter.on.then(|| self.matter.code.clone()),
            zigbee: self.zigbee,
            wwn_boiler: self.wwn_boiler,
            entry_point: self.entry_point,
            doom: self.doom,
            external_temperature: self.external_temperature,
            ssh: self.ssh,
            ssh_public_key: self.ssh.then(|| self.ssh_key.path.clone()).flatten(),
            ssh_key_only: self.ssh && !self.ssh_password_login,
            usb_shell: self.usb_shell,
        }
    }

    pub fn config(&self) -> Result<Config, String> {
        let options = self.inputs().build().map_err(|e| e.to_string())?;
        let data_dir = default_data_dir();
        Ok(Config {
            options,
            omap: self.device,
            slot: self.slot,
            dry_run: self.dry_run,
            backup: self.backup_volume,
            entry_point: self.entry_point,
            wait_usb: WAIT_USB,
            wait_shell: Duration::from_secs(180),
            work_dir: data_dir.join("runs"),
            data_dir,
        })
    }
}

impl Form {
    /// The run only reads: it writes nothing to the unit.
    pub fn dry_run(&self) -> bool {
        self.dry_run
    }

    /// A file picker is still open: its choice would be missed by a run
    /// started now.
    pub fn dialog_pending(&self) -> bool {
        [&self.mqtt_ca, &self.overlay, &self.ssh_key].iter().any(|pick| pick.pending.is_some())
    }

    fn drain_dialogs(&mut self) {
        for pick in [&mut self.mqtt_ca, &mut self.overlay, &mut self.ssh_key] {
            pick.drain();
        }
    }

    /// Whether the Home Assistant step can be left: with MQTT on, a broker
    /// must be named.
    pub fn home_assistant_done(&self) -> bool {
        !self.mqtt.on || !self.mqtt.value.trim().is_empty()
    }

    /// The Home Assistant step: the broker, and updates through it.
    pub fn home_assistant_ui(&mut self, ui: &mut egui::Ui) {
        self.drain_dialogs();
        let on = crate::option_card(ui, &mut self.mqtt.on, true, crate::tr!("setup.mqtt"), crate::tr!("setup.mqtt_about"), |ui| {
            // The broker across the card, the sign-in side by side under it.
            let width = ui.available_width();
            let half = (width - 14.0) / 2.0;
            crate::field(ui, crate::tr!("patch.mqtt.broker"),
                         egui::TextEdit::singleline(&mut self.mqtt.value).hint_text(crate::tr!("setup.mqtt_broker_example")), width)
                .on_hover_text(crate::tr!("patch.mqtt.broker_hint"));
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                crate::field(ui, crate::tr!("patch.mqtt.user"),
                             egui::TextEdit::singleline(&mut self.mqtt_user).hint_text(crate::tr!("setup.optional")), half);
                crate::field(ui, crate::tr!("patch.mqtt.password"),
                             egui::TextEdit::singleline(&mut self.mqtt_password).password(true).hint_text(crate::tr!("setup.optional")), half);
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                crate::switch(ui, &mut self.mqtt_tls, true).on_hover_text(crate::tr!("patch.mqtt.tls_hint"));
                ui.label(crate::tr!("setup.mqtt_tls"));
                if self.mqtt_tls {
                    self.mqtt_ca.ui(ui, crate::tr!("patch.mqtt.ca_certificate"), crate::tr!("patch.file.certificate"), &["pem", "crt"]);
                }
            });
        });
        let off_note = |ui: &mut egui::Ui| crate::inset(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(crate::tr!("setup.updates_off")).size(13.0)).wrap());
            ui.add(egui::Label::new(egui::RichText::new(crate::tr!("setup.updates_off_reflash")).size(13.0).color(crate::WARN)).wrap());
        });
        let updates = crate::option_card(ui, &mut self.updates, on, crate::tr!("setup.updates"), crate::tr!("setup.updates_about"), off_note);
        if !on {
            crate::muted_line(ui, crate::tr!("setup.updates_needs_mqtt"));
        } else if !updates {
            off_note(ui);
        }
    }

    /// The features step: privacy, other controllers, remote access, and
    /// Doom last.
    pub fn features_ui(&mut self, ui: &mut egui::Ui) {
        self.drain_dialogs();
        // First, as it decides whether the Nest heats at all: a Heat Link
        // model (UK and Europe) without its Heat Link stops on H71/H74.
        crate::group(ui, crate::tr!("setup.group.heating"));
        ui.label(egui::RichText::new(crate::tr!("setup.heating_question")).size(14.5));
        ui.add_space(8.0);
        if crate::choice_card(ui, !self.wwn_boiler, crate::tr!("setup.heat_link"), crate::tr!("setup.heat_link_about"), None).clicked() {
            self.wwn_boiler = false;
        }
        ui.add_space(10.0);
        if crate::choice_card(ui, self.wwn_boiler, crate::tr!("setup.no_heat_link"), crate::tr!("setup.no_heat_link_about"),
                              Some(crate::tr!("patch.wwn_boiler"))).clicked() {
            self.wwn_boiler = true;
        }
        ui.add_space(6.0);
        crate::muted_line(ui, crate::tr!("setup.heating_later"));
        crate::group(ui, crate::tr!("setup.group.privacy"));
        let block = crate::option_card(ui, &mut self.block_cloud, true, crate::tr!("patch.block_cloud"), crate::tr!("setup.block_cloud_about"), |ui| {
            ui.horizontal(|ui| {
                ui.label(crate::tr!("setup.time_server"));
                ui.add(egui::TextEdit::singleline(&mut self.ntp.value).hint_text(DEFAULT_NTP_SERVER).desired_width(FIELD_WIDTH));
                ui.label(egui::RichText::new(crate::tr!("setup.time_server_why")).color(crate::MUTED).size(13.0));
            });
        });
        // time.nest.com is blocked too, so time has to come from elsewhere.
        if block {
            self.ntp.on = true;
            if self.ntp.value.is_empty() {
                self.ntp.value = DEFAULT_NTP_SERVER.into();
            }
        } else {
            self.ntp.on = false;
        }

        crate::group(ui, crate::tr!("setup.group.controllers"));
        let matter_was = self.matter.on;
        crate::option_card(ui, &mut self.matter.on, true, crate::tr!("patch.matter"), crate::tr!("setup.matter_about"), |ui| {
            ui.horizontal(|ui| {
                ui.label(crate::tr!("patch.matter.pairing_code"));
                ui.add(egui::TextEdit::singleline(&mut self.matter.code).hint_text(DEFAULT_MATTER_CODE)
                    .desired_width(160.0)).on_hover_text(crate::tr!("patch.matter.pairing_code_hint"));
            });
            crate::checkbox(ui, !self.zigbee, &mut self.matter_thread, crate::tr!("patch.matter.thread"))
                .on_hover_text(if self.zigbee { crate::tr!("patch.matter.thread_disabled_hint") } else { crate::tr!("patch.matter.thread_hint") });
        });
        if matter_was && !self.matter.on {
            self.matter_thread = false;
        }
        let thread = self.matter_thread;
        crate::option_card(ui, &mut self.zigbee, !thread, crate::tr!("setup.zigbee"), crate::tr!("setup.zigbee_about"), |_| {});
        if thread {
            crate::muted_line(ui, crate::tr!("patch.zigbee_disabled_hint"));
        }
        // The radio a Heat Link talks over goes to nestlocal.
        if self.holds_radio() && !self.wwn_boiler {
            ui.add(egui::Label::new(egui::RichText::new(crate::tr!("setup.radio_needs_no_heat_link")).size(13.0).color(crate::WARN)).wrap());
            ui.add_space(4.0);
        }

        crate::group(ui, crate::tr!("setup.group.access"));
        crate::option_card(ui, &mut self.ssh, true, crate::tr!("setup.ssh"), crate::tr!("setup.ssh_about"), |ui| {
            egui::Grid::new("ssh_fields").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
                ui.label(crate::tr!("setup.ssh_key"));
                ui.horizontal(|ui| self.ssh_key.ui(ui, crate::tr!("patch.file.choose"), crate::tr!("patch.file.openssh_public_key"), &["pub"]));
                ui.end_row();
            });
            crate::checkbox(ui, true, &mut self.ssh_password_login, crate::tr!("setup.ssh_password_login"))
                .on_hover_text(crate::tr!("setup.ssh_password_login_hint"));
            if self.ssh_password_login {
                ui.horizontal(|ui| {
                    ui.label(crate::tr!("patch.mqtt.password"));
                    ui.add(egui::TextEdit::singleline(&mut self.root_password).password(true)
                        .hint_text(crate::tr!("setup.ssh_password_hint")).desired_width(FIELD_WIDTH));
                });
                if self.root_password.is_empty() {
                    ui.add(egui::Label::new(egui::RichText::new(crate::tr!("setup.ssh_default_password_warning")).color(crate::WARN).size(13.0)).wrap());
                }
            }
            crate::inset(ui, |ui| {
                ui.label(egui::RichText::new(crate::tr!("setup.ssh_key_what")).font(crate::medium(14.0)));
                ui.add(egui::Label::new(egui::RichText::new(crate::tr!("setup.ssh_key_explain")).size(13.0)).wrap());
                ui.horizontal_wrapped(|ui| {
                    if crate::small_accent_button(ui, crate::tr!("setup.ssh_create")).clicked() {
                        match create_ssh_key(&default_ssh_key()) {
                            Ok(public) => {
                                self.ssh_key.path = Some(public);
                                self.ssh_key_error = None;
                            }
                            Err(e) => self.ssh_key_error = Some(e),
                        }
                    }
                    ui.label(egui::RichText::new(crate::tr!("setup.ssh_create_where")).color(crate::MUTED).size(13.0));
                });
                if let Some(error) = &self.ssh_key_error {
                    ui.add(egui::Label::new(egui::RichText::new(error).color(crate::BAD).size(13.0)).wrap());
                }
                ui.label(egui::RichText::new(crate::tr!("setup.ssh_login")).size(13.0));
                crate::code_line(ui, &ssh_command(self.ssh_key.path.as_deref()));
            });
        });
        crate::option_card(ui, &mut self.usb_shell, true, crate::tr!("setup.usb_console"), crate::tr!("setup.usb_console_about"), |_| {});

        crate::group(ui, crate::tr!("setup.group.fun"));
        crate::option_card(ui, &mut self.doom, true, crate::tr!("patch.doom"), crate::tr!("setup.doom_about"), |ui| {
            crate::inset(ui, |ui| {
                ui.label(egui::RichText::new(crate::tr!("setup.doom_how_title")).font(crate::medium(14.0)));
                crate::numbered(ui, &[crate::tr!("setup.doom_how_open"), crate::tr!("setup.doom_how_play"), crate::tr!("setup.doom_how_quit")]);
                ui.label(egui::RichText::new(crate::tr!("setup.doom_heating")).color(crate::WARN).size(13.0));
            });
        });
    }

    /// Whether the features step can be left: SSH needs a way in, a key or
    /// password login (with the default password if none is set; the
    /// person's choice, warned of).
    pub fn features_done(&self) -> bool {
        !self.ssh || self.ssh_password_login || self.ssh_key.path.is_some()
    }

    /// What the run will do, a line per choice, for the review step.
    pub fn summary(&self) -> Vec<(&'static str, String)> {
        let on_off = |on: bool| if on { crate::tr!("setup.on") } else { crate::tr!("setup.off") }.to_string();
        let mut rows = vec![(crate::tr!("setup.mqtt"), if self.mqtt.on { self.mqtt.value.trim().to_string() } else { on_off(false) })];
        if self.mqtt.on {
            rows.push((crate::tr!("setup.updates"), on_off(self.updates)));
        }
        rows.push((crate::tr!("patch.block_cloud"), on_off(self.block_cloud)));
        if self.ntp.on {
            rows.push((crate::tr!("setup.time_server"), self.ntp.value.clone()));
        }
        let mut extras: Vec<&str> = Vec::new();
        if self.doom { extras.push(crate::tr!("patch.doom")); }
        if self.matter.on { extras.push(if self.matter_thread { crate::tr!("setup.matter_thread_short") } else { crate::tr!("patch.matter") }); }
        if self.zigbee { extras.push(crate::tr!("setup.zigbee")); }
        if self.wwn_boiler { extras.push(crate::tr!("patch.wwn_boiler")); }
        if self.ssh { extras.push(crate::tr!("setup.ssh")); }
        if self.usb_shell { extras.push(crate::tr!("setup.usb_console")); }
        rows.push((crate::tr!("setup.extras"), if extras.is_empty() { crate::tr!("setup.none").to_string() } else { extras.join(" · ") }));
        if self.dry_run {
            rows.push((crate::tr!("patch.dry_run"), crate::tr!("setup.dry_run_summary").to_string()));
        }
        rows
    }

    /// The developer options: the CLI's options a run rarely needs.
    /// The device a run is for: the one asked for, else the one on the bus.
    fn known_device(&self, bus: &Bus) -> Option<Device> {
        self.device.or(match bus {
            Bus::One(seen) => Some(seen.device),
            _ => None,
        })
    }

    /// Puts the entry point back to auto when the device changes for
    /// another; a unit unplugged, or several on the bus, is no change.
    pub fn track_device(&mut self, bus: &Bus) {
        if let Some(device) = self.known_device(bus) {
            if self.last_device.is_some_and(|last| last != device) {
                self.entry_point = EntryPoint::Auto;
            }
            self.last_device = Some(device);
        }
    }

    /// The developer options, folded away on the review step.
    pub fn developer_ui(&mut self, ui: &mut egui::Ui, bus: &Bus) {
        self.drain_dialogs();
        // With nestlocal, the overlay and the cloud are its.
        let nestlocal_hint = crate::tr!("patch.nestlocal_owns_overlay_cloud");
        let nestlocal = self.nestlocal();
        let grid = |id: &str| egui::Grid::new(id).num_columns(2).min_col_width(ADVANCED_LABEL_WIDTH).spacing([12.0, 6.0]);
        crate::checkbox(ui, true, &mut self.dry_run, crate::tr!("patch.dry_run")).on_hover_text(crate::tr!("patch.dry_run_hint"));
        crate::checkbox(ui, true, &mut self.backup_volume, crate::tr!("patch.backup_volume")).on_hover_text(crate::tr!("patch.backup_volume_hint"));
        crate::checkbox(ui, true, &mut self.backplate_sim, crate::tr!("patch.backplate_sim"))
            .on_hover_text(crate::tr!("patch.backplate_sim_hint"));
        grid("developer_run").show(ui, |ui| {
            ui.label(crate::tr!("patch.boot_slot"));
            crate::dropdown("slot", CONTROL_WIDTH)
                .selected_text(self.slot.map_or(crate::tr!("patch.auto").to_string(), |s| s.to_string())).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.slot, None, crate::tr!("patch.auto"));
                    for slot in [Slot::Root0, Slot::Root1] {
                        ui.selectable_value(&mut self.slot, Some(slot), slot.to_string());
                    }
                }).response.on_hover_text(crate::tr!("patch.boot_slot_hint"));
            ui.end_row();
            ui.label(crate::tr!("patch.device"));
            crate::dropdown("device", CONTROL_WIDTH)
                .selected_text(self.device.map_or(crate::tr!("patch.auto"), Device::name)).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.device, None, crate::tr!("patch.auto"));
                    for device in [Device::Diamond, Device::J49, Device::Diamond3] {
                        ui.selectable_value(&mut self.device, Some(device), device.name());
                    }
                }).response.on_hover_text(crate::tr!("patch.device_hint"));
            ui.end_row();
            let gen3 = self.known_device(bus) == Some(Device::Diamond3);
            ui.label(crate::tr!("patch.entry_point")).on_hover_text(crate::tr!("patch.entry_point_hint"));
            // Greyed without a gen 3 on the bus, keeping the choice: the unit
            // may only have been unplugged, and other generations ignore it.
            ui.add_enabled_ui(gen3, |ui| {
                crate::dropdown("technique", CONTROL_WIDTH)
                    .selected_text(entry_point_label(self.entry_point)).show_ui(ui, |ui| {
                        for entry_point in [EntryPoint::Auto, EntryPoint::OfficialLoader, EntryPoint::Cve20177932, EntryPoint::Cve20177936] {
                            ui.selectable_value(&mut self.entry_point, entry_point, entry_point_label(entry_point));
                        }
                    }).response.on_hover_text(crate::tr!("patch.entry_point_choice_hint"));
            }).response.on_disabled_hover_text(crate::tr!("patch.entry_point_disabled_hint"));
            ui.end_row();
            ui.label(crate::tr!("patch.overlay"));
            ui.add_enabled_ui(!nestlocal, |ui| {
                ui.horizontal(|ui| self.overlay.ui_sized(ui, crate::tr!("patch.file.choose"), crate::tr!("patch.file.tar_archive"), &["tar"], CONTROL_WIDTH));
            }).response.on_disabled_hover_text(nestlocal_hint);
            ui.end_row();
        });
        ui.add_enabled_ui(!nestlocal, |ui| {
            crate::checkbox(ui, true, &mut self.cloud_url.on, crate::tr!("patch.cloud"))
                .on_hover_text(crate::tr!("patch.cloud_hint"));
            if self.cloud_url.on {
                grid("developer_cloud").show(ui, |ui| {
                    ui.label(crate::tr!("patch.cloud_url"));
                    ui.add(egui::TextEdit::singleline(&mut self.cloud_url.value).hint_text("https://frontdoor.example")
                        .desired_width(CONTROL_WIDTH));
                    ui.end_row();
                });
            }
        }).response.on_disabled_hover_text(nestlocal_hint);
    }
}

/// Text fields in the steps' cards.
const FIELD_WIDTH: f32 = 260.0;
/// The developer options' label column, and their drop-downs and buttons,
/// all one width.
const ADVANCED_LABEL_WIDTH: f32 = 84.0;
const CONTROL_WIDTH: f32 = 180.0;

/// Where Create a key for me puts the key: ~/.ssh/fledge.
fn default_ssh_key() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    home.join(".ssh").join("fledge")
}

/// Makes an SSH key pair at `private` with ssh-keygen, or keeps the one
/// there, and returns its public half. ECDSA: the gen 3's OpenSSH 6.0
/// predates Ed25519.
fn create_ssh_key(private: &std::path::Path) -> Result<PathBuf, String> {
    let mut public = private.as_os_str().to_owned();
    public.push(".pub");
    let public = PathBuf::from(public);
    if private.exists() && public.exists() {
        return Ok(public);
    }
    if let Some(dir) = private.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let output = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ecdsa", "-b", "256", "-N", "", "-C", "fledge", "-f"]).arg(private)
        .output().map_err(|e| crate::tr!("setup.ssh_keygen_missing", error = e))?;
    if !output.status.success() || !public.exists() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(public)
}

/// The command that logs in with the key at `public`'s private half.
fn ssh_command(public: Option<&std::path::Path>) -> String {
    let key = public.map_or_else(|| "~/.ssh/fledge".to_string(), |path| {
        let private = path.with_extension("");
        let home = std::env::var_os("HOME").map(PathBuf::from);
        match home.as_deref().and_then(|home| private.strip_prefix(home).ok()) {
            Some(rest) => format!("~/{}", rest.display()),
            None => private.display().to_string(),
        }
    });
    format!("ssh -i {key} root@{}", crate::tr!("setup.ssh_address"))
}

/// The entry-point labels offered by the GUI, independent of CLI option names.
fn entry_point_label(entry_point: EntryPoint) -> &'static str {
    match entry_point {
        EntryPoint::Auto => crate::tr!("patch.auto"),
        EntryPoint::Cve20177932 => crate::tr!("patch.technique.jump_certificates"),
        EntryPoint::Cve20177936 => crate::tr!("patch.technique.spray_stack"),
        EntryPoint::OfficialLoader => crate::tr!("patch.technique.official_loader"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_need_home_assistant_and_ssh_needs_a_key_or_password_login() {
        let mut form = Form::recommended();
        form.mqtt.value = "broker".into();
        assert!(form.inputs().updates, "on by default with Home Assistant");
        form.mqtt.on = false;
        assert!(!form.inputs().updates, "no updates without MQTT");
        form.ssh = true;
        assert!(!form.features_done(), "keys only by default, and no key yet");
        assert!(form.inputs().ssh_key_only);
        form.ssh_password_login = true;
        assert!(form.features_done(), "password login, with the default password if none is set");
        assert!(!form.inputs().ssh_key_only);
    }

    #[test]
    fn create_a_key_makes_an_ecdsa_pair_once_and_keeps_it() {
        if std::process::Command::new("ssh-keygen").arg("-?").output().is_err() {
            return; // no OpenSSH on this machine
        }
        let dir = std::env::temp_dir().join(format!("fledge-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let private = dir.join("nested").join("fledge");
        let public = create_ssh_key(&private).unwrap();
        let text = std::fs::read_to_string(&public).unwrap();
        assert!(text.starts_with("ecdsa-sha2-nistp256 "), "the gen 3's OpenSSH 6.0 has no Ed25519: {text}");
        assert!(fledge_core::patch::ssh_public_key(text.as_bytes()).is_ok(), "the patch takes it");
        let modified = std::fs::metadata(&private).unwrap().modified().unwrap();
        assert_eq!(create_ssh_key(&private).unwrap(), public);
        assert_eq!(std::fs::metadata(&private).unwrap().modified().unwrap(), modified, "an existing key is kept");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_login_command_names_the_chosen_key() {
        assert_eq!(ssh_command(None), "ssh -i ~/.ssh/fledge root@<thermostat's IP address>");
        assert_eq!(ssh_command(Some(std::path::Path::new("/keys/nest.pub"))), "ssh -i /keys/nest root@<thermostat's IP address>");
    }

    #[test]
    fn the_entry_point_goes_back_to_auto_only_when_the_device_changes() {
        use crate::bus::Seen;
        use fledge_host::{device, diamond3::usb};
        let one = |id| Bus::One(Seen { device: device::identify(id).unwrap(), mode: "", port: "1".into(), id });
        let gen3 = one(usb::SDP);
        let gen2 = Bus::One(Seen { device: Device::J49, mode: "", port: "1".into(), id: (0, 0) });
        let mut form = Form::default();
        form.track_device(&gen3);
        form.entry_point = EntryPoint::Cve20177936;
        for bus in [Bus::Nothing, Bus::Several(Vec::new()), one(usb::RECOVERY), one(usb::APP_ACM), one(usb::INITRAMFS), gen3.clone()] {
            form.track_device(&bus);
            assert_eq!(form.entry_point, EntryPoint::Cve20177936, "{bus:?}");
        }
        form.track_device(&Bus::Nothing);
        form.track_device(&gen2);
        assert_eq!(form.entry_point, EntryPoint::Auto, "swapped while unplugged");
        form.track_device(&gen3);
        form.entry_point = EntryPoint::Cve20177936;
        form.device = Some(Device::J49);
        form.track_device(&gen3);
        assert_eq!(form.entry_point, EntryPoint::Auto, "another device asked for");
    }

    #[test]
    fn form_maps_only_enabled_options() {
        let mut form = Form { root_password: "s3cret".into(), backplate_sim: true, ..Form::default() };
        form.mqtt = Toggle { on: true, value: "broker".into() };
        form.mqtt_user = "u".into();
        form.mqtt_tls = true;
        let inputs = form.inputs();
        assert_eq!(inputs.mqtt.as_deref(), Some("broker"));
        assert_eq!(inputs.mqtt_user.as_deref(), Some("u"));
        assert!(inputs.mqtt_tls);
        assert_eq!(inputs.root_password.as_deref(), Some("s3cret"));
        assert!(inputs.backplate_sim);
        // Toggles that are off contribute nothing.
        assert!(inputs.ntp_server.is_none());

        let bare = Form::default().inputs();
        assert!(bare.mqtt.is_none() && bare.root_password.is_none() && !bare.backplate_sim);
    }

    #[test]
    fn mqtt_fields_are_dropped_when_mqtt_is_off() {
        let form = Form { mqtt_user: "u".into(), mqtt_tls: true, ..Form::default() };
        let inputs = form.inputs();
        assert!(inputs.mqtt.is_none() && inputs.mqtt_user.is_none() && !inputs.mqtt_tls);
        // build() would otherwise reject user-without-broker; here it is clean.
        assert!(inputs.build().is_ok());
    }

    #[test]
    fn block_cloud_moves_time_off_time_nest_com() {
        let mut form = Form { block_cloud: true, ..Form::default() };
        let options = form.config().unwrap().options;
        assert!(options.block_cloud && options.ntp_server.as_deref() == Some(""));
        form.ntp = Toggle { on: true, value: "ntp.lan".into() };
        assert_eq!(form.config().unwrap().options.ntp_server.as_deref(), Some("ntp.lan"));
    }

    #[test]
    fn thread_is_opt_in_and_requires_matter_and_exclusive_radio() {
        let mut form = Form::default();
        assert!(form.config().unwrap().options.thread.is_none());
        form.matter.on = true;
        assert!(form.config().unwrap().options.thread.is_none());
        form.matter_thread = true;
        let options = form.config().unwrap().options;
        assert!(options.thread.unwrap().files.iter().any(|(name, _, _)| name == "nestlabs/sbin/ot-daemon"));
        form.zigbee = true;
        assert!(form.config().err().unwrap().contains("cannot share the radio"));
        form.matter.on = false;
        assert!(form.config().unwrap().options.thread.is_none());
    }

    #[test]
    fn matter_takes_the_pairing_code_shown() {
        let mut form = Form::default();
        assert_eq!(form.matter.code, DEFAULT_MATTER_CODE, "prefilled");
        assert!(form.config().unwrap().options.matter.is_none(), "off by default");
        form.matter.on = true;
        let matter = form.config().unwrap().options.matter.unwrap();
        assert_eq!(matter.pairing_code(), DEFAULT_MATTER_CODE);
        form.matter.code = "1559-813-5763".into();
        assert_eq!(form.config().unwrap().options.matter.unwrap().passcode, 58596013);
        form.matter.code = "1559-813-5764".into();
        assert!(form.config().err().is_some_and(|e| e.contains("check digit")));
    }

    #[test]
    fn config_runs_the_real_write_on_the_boot_order_slot() {
        let config = Form::default().config().unwrap();
        assert_eq!((config.slot, config.omap), (None, None));
        assert!(!config.dry_run && !config.backup);
    }

    #[test]
    fn the_advanced_section_maps_to_the_cli_options() {
        let mut form = Form { dry_run: true, backup_volume: true, doom: true, slot: Some(Slot::Root1),
                              device: Some(Device::J49), ..Form::default() };
        form.ssh_key.path = Some("key.pub".into());
        let config = form.config().unwrap();
        assert!(config.dry_run && config.backup);
        assert_eq!((config.slot, config.omap), (Some(Slot::Root1), Some(Device::J49)));
        assert!(config.options.doom.is_some());
        assert!(form.inputs().ssh_public_key.is_none(), "the key only goes with the SSH server");
        form.ssh = true;
        assert_eq!(form.inputs().ssh_public_key.as_deref(), Some(std::path::Path::new("key.pub")));
        form.cloud_url = Toggle { on: true, value: "https://cloud.example".into() };
        form.overlay.path = Some("extra.tar".into());
        let inputs = form.inputs();
        assert!(inputs.cloud_url.is_none() && inputs.overlay.is_none(), "doom installs nestlocal, which has its own");
        form.doom = false;
        let inputs = form.inputs();
        assert_eq!(inputs.cloud_url.as_deref(), Some("https://cloud.example"));
        assert_eq!(inputs.overlay.as_deref(), Some(std::path::Path::new("extra.tar")));
        form.doom = true;
        form.wwn_boiler = true;
        assert!(form.inputs().wwn_boiler, "with or without a radio nestlocal holds");
        form.zigbee = true;
        assert!(form.inputs().wwn_boiler);
        form.ssh = false;
        assert!(form.config().unwrap().options.wwn_boiler);
        form.zigbee = false;
        form.wwn_boiler = false;
        form.entry_point = EntryPoint::Cve20177936;
        assert_eq!(form.inputs().entry_point, EntryPoint::Cve20177936);
        assert_eq!(form.config().unwrap().entry_point, EntryPoint::Cve20177936);
        form.entry_point = EntryPoint::Auto;
        assert!(!form.dialog_pending());
        form.cloud_url.on = false;
        form.overlay.path = None;
    }
}
