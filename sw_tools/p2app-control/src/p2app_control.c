#include <gtk/gtk.h>
#include <libayatana-appindicator/app-indicator.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>

static const char *UNIT = "p2app.service";
static const char *ICON_NAME = "p2appcontrol";
static const char *ICON_PIXMAP_DIR = "/usr/local/share/pixmaps";
static const char *ICON_PIXMAP_PATH = "/usr/local/share/pixmaps/p2appcontrol.png";
static const char *ICON_THEME_DIR = "/usr/local/share/icons/hicolor/32x32/apps";
static const char *ICON_THEME_PATH = "/usr/local/share/icons/hicolor/32x32/apps/p2appcontrol.png";
static const char *APP_INFO_HELPER = "/usr/local/bin/saturn-g2-version-info.sh";

typedef struct {
    gboolean tray_mode;
    GtkWidget *win;
    GtkWidget *status_label;
    GtkWidget *boot_label;
    GtkWidget *btn_start;
    GtkWidget *btn_stop;
    GtkWidget *btn_restart;
    GtkWidget *btn_enable_boot;
    GtkWidget *btn_disable_boot;
    GtkWidget *btn_show_info;
    GtkWidget *btn_quit;
    AppIndicator *indicator;
    GtkWidget *tray_menu;
    GtkWidget *tray_show_item;
    GtkWidget *tray_start_item;
    GtkWidget *tray_stop_item;
    GtkWidget *tray_restart_item;
    GtkWidget *tray_show_info_item;
    GtkWidget *tray_enable_boot_item;
    GtkWidget *tray_disable_boot_item;
} UI;

static gboolean run_capture_status(const char *cmd, char *out, gsize outlen, int *exit_code) {
    gchar *stdout_buf = NULL;
    gchar *stderr_buf = NULL;
    gint status = 0;
    GError *err = NULL;

    gboolean ok = g_spawn_command_line_sync(cmd, &stdout_buf, &stderr_buf, &status, &err);
    if (!ok || err) {
        if (out && outlen) {
            g_strlcpy(out, err ? err->message : "spawn failed", outlen);
        }
        if (err) {
            g_error_free(err);
        }
        g_free(stdout_buf);
        g_free(stderr_buf);
        return FALSE;
    }

    if (exit_code) {
        if (WIFEXITED(status)) {
            *exit_code = WEXITSTATUS(status);
        } else {
            *exit_code = -1;
        }
    }

    if (out && outlen) {
        const char *src = (stdout_buf && *stdout_buf) ? stdout_buf : (stderr_buf ? stderr_buf : "");
        g_strlcpy(out, src, outlen);
    }

    g_free(stdout_buf);
    g_free(stderr_buf);
    return TRUE;
}

static gboolean run_capture(const char *cmd, char *out, gsize outlen) {
    return run_capture_status(cmd, out, outlen, NULL);
}

static void get_service_state(char *out, gsize outlen) {
    char cmd[256];
    g_strlcpy(out, "unknown", outlen);
    snprintf(cmd, sizeof(cmd), "systemctl is-active %s", UNIT);
    if (!run_capture(cmd, out, outlen)) {
        return;
    }
    g_strstrip(out);
}

static void get_enable_state(char *out, gsize outlen) {
    char cmd[256];
    g_strlcpy(out, "unknown", outlen);
    snprintf(cmd, sizeof(cmd), "systemctl is-enabled %s 2>/dev/null || true", UNIT);
    if (!run_capture(cmd, out, outlen)) {
        return;
    }
    g_strstrip(out);
    if (!*out) {
        g_strlcpy(out, "unknown", outlen);
    }
}

static gboolean is_active_state(const char *state) {
    return g_strcmp0(state, "active") == 0;
}

static gboolean is_enabled_state(const char *state) {
    return g_strcmp0(state, "enabled") == 0 ||
           g_strcmp0(state, "enabled-runtime") == 0 ||
           g_strcmp0(state, "linked") == 0 ||
           g_strcmp0(state, "linked-runtime") == 0 ||
           g_strcmp0(state, "alias") == 0;
}

static gboolean have_custom_theme_icon(void) {
    return g_file_test(ICON_THEME_PATH, G_FILE_TEST_EXISTS);
}

static gboolean have_custom_pixmap_icon(void) {
    return g_file_test(ICON_PIXMAP_PATH, G_FILE_TEST_EXISTS);
}

static gboolean have_custom_icon(void) {
    return have_custom_theme_icon() || have_custom_pixmap_icon();
}

static const char *custom_icon_file(void) {
    if (have_custom_theme_icon()) {
        return ICON_THEME_PATH;
    }
    if (have_custom_pixmap_icon()) {
        return ICON_PIXMAP_PATH;
    }
    return NULL;
}

static const char *custom_icon_lookup_dir(void) {
    if (have_custom_theme_icon()) {
        return ICON_THEME_DIR;
    }
    if (have_custom_pixmap_icon()) {
        return ICON_PIXMAP_DIR;
    }
    return NULL;
}

static void privileged_systemctl(const char *verb) {
    char cmd[256];
    int rc;

    /* Prefer the installer-provided sudoers rule; fall back to pkexec on older installs. */
    snprintf(cmd, sizeof(cmd), "sudo -n /bin/systemctl %s %s >/dev/null 2>&1", verb, UNIT);
    rc = system(cmd);
    if (rc == 0) {
        return;
    }

    snprintf(cmd, sizeof(cmd), "pkexec /bin/systemctl %s %s", verb, UNIT);
    (void)system(cmd);
}

static void update_window_visibility_item(UI *ui) {
    if (!ui->tray_show_item) {
        return;
    }
    gtk_menu_item_set_label(GTK_MENU_ITEM(ui->tray_show_item),
                            gtk_widget_get_visible(ui->win) ? "Hide Control Window" : "Show Control Window");
}

static void show_window(UI *ui) {
    gtk_widget_show_all(ui->win);
    gtk_window_present(GTK_WINDOW(ui->win));
    update_window_visibility_item(ui);
}

static void hide_window(UI *ui) {
    gtk_widget_hide(ui->win);
    update_window_visibility_item(ui);
}

static void on_start(GtkWidget *unused, gpointer data) {
    (void)unused;
    (void)data;
    privileged_systemctl("start");
}

static void on_stop(GtkWidget *unused, gpointer data) {
    (void)unused;
    (void)data;
    privileged_systemctl("stop");
}

static void on_restart(GtkWidget *unused, gpointer data) {
    (void)unused;
    (void)data;
    privileged_systemctl("restart");
}

static void on_enable_boot(GtkWidget *unused, gpointer data) {
    (void)unused;
    (void)data;
    privileged_systemctl("enable");
}

static void on_disable_boot(GtkWidget *unused, gpointer data) {
    (void)unused;
    (void)data;
    privileged_systemctl("disable");
}

static gboolean collect_app_info(char *out, gsize outlen) {
    char cmd[512];
    int rc = -1;

    if (!out || outlen == 0) {
        return FALSE;
    }

    out[0] = '\0';

    snprintf(cmd, sizeof(cmd), "sudo -n %s", APP_INFO_HELPER);
    if (run_capture_status(cmd, out, outlen, &rc) && rc == 0 && *out) {
        return TRUE;
    }

    snprintf(cmd, sizeof(cmd), "%s", APP_INFO_HELPER);
    if (run_capture_status(cmd, out, outlen, &rc) && rc == 0 && *out) {
        return TRUE;
    }

    if (!*out) {
        g_strlcpy(out,
                  "Unable to collect app info.\n"
                  "Check that saturn-g2-version-info.sh is installed and readable.",
                  outlen);
    }
    return FALSE;
}

static void app_info_copy_to_clipboard(const char *text) {
    GtkClipboard *clipboard = gtk_clipboard_get(GDK_SELECTION_CLIPBOARD);
    if (clipboard && text) {
        gtk_clipboard_set_text(clipboard, text, -1);
    }
}

static void show_app_info_dialog(UI *ui) {
    GtkWidget *parent = NULL;
    GtkWidget *dialog;
    GtkWidget *content;
    GtkWidget *scroller;
    GtkWidget *text_view;
    GtkTextBuffer *buffer;
    char info[32768];
    gint response;

    if (gtk_widget_get_visible(ui->win)) {
        parent = ui->win;
    }

    collect_app_info(info, sizeof(info));

    dialog = gtk_dialog_new_with_buttons("Saturn App / Firmware Info",
                                         parent ? GTK_WINDOW(parent) : NULL,
                                         GTK_DIALOG_MODAL | GTK_DIALOG_DESTROY_WITH_PARENT,
                                         "Refresh", 1,
                                         "Copy", 2,
                                         "Close", GTK_RESPONSE_CLOSE,
                                         NULL);
    gtk_window_set_default_size(GTK_WINDOW(dialog), 860, 560);

    content = gtk_dialog_get_content_area(GTK_DIALOG(dialog));
    scroller = gtk_scrolled_window_new(NULL, NULL);
    gtk_widget_set_hexpand(scroller, TRUE);
    gtk_widget_set_vexpand(scroller, TRUE);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(scroller),
                                   GTK_POLICY_AUTOMATIC,
                                   GTK_POLICY_AUTOMATIC);
    gtk_container_add(GTK_CONTAINER(content), scroller);

    text_view = gtk_text_view_new();
    gtk_text_view_set_editable(GTK_TEXT_VIEW(text_view), FALSE);
    gtk_text_view_set_cursor_visible(GTK_TEXT_VIEW(text_view), FALSE);
    gtk_text_view_set_wrap_mode(GTK_TEXT_VIEW(text_view), GTK_WRAP_NONE);
    gtk_text_view_set_monospace(GTK_TEXT_VIEW(text_view), TRUE);
    gtk_container_add(GTK_CONTAINER(scroller), text_view);

    buffer = gtk_text_view_get_buffer(GTK_TEXT_VIEW(text_view));
    gtk_text_buffer_set_text(buffer, info, -1);

    gtk_widget_show_all(dialog);

    for (;;) {
        response = gtk_dialog_run(GTK_DIALOG(dialog));
        if (response == 1) {
            collect_app_info(info, sizeof(info));
            gtk_text_buffer_set_text(buffer, info, -1);
            continue;
        }
        if (response == 2) {
            GtkTextIter start;
            GtkTextIter end;
            gchar *text;

            gtk_text_buffer_get_bounds(buffer, &start, &end);
            text = gtk_text_buffer_get_text(buffer, &start, &end, FALSE);
            app_info_copy_to_clipboard(text);
            g_free(text);
            continue;
        }
        break;
    }

    gtk_widget_destroy(dialog);
}

static void on_show_info(GtkWidget *unused, gpointer data) {
    (void)unused;
    show_app_info_dialog((UI *)data);
}

static void on_quit(GtkWidget *unused, gpointer data) {
    (void)unused;
    (void)data;
    gtk_main_quit();
}

static void on_tray_show_toggle(GtkWidget *unused, gpointer data) {
    (void)unused;
    UI *ui = (UI *)data;
    if (gtk_widget_get_visible(ui->win)) {
        hide_window(ui);
    } else {
        show_window(ui);
    }
}

static gboolean on_window_delete(GtkWidget *widget, GdkEvent *event, gpointer data) {
    (void)widget;
    (void)event;
    UI *ui = (UI *)data;
    if (ui->tray_mode) {
        hide_window(ui);
        return TRUE;
    }
    return FALSE;
}

static void on_window_destroy(GtkWidget *widget, gpointer data) {
    (void)widget;
    UI *ui = (UI *)data;
    if (!ui->tray_mode) {
        gtk_main_quit();
    }
}

static void update_tray_state(UI *ui, const char *state, gboolean active) {
    const char *icon_name = "media-playback-stop";
    const char *status_desc = "P2_app STOPPED";

    if (!ui->indicator) {
        return;
    }

    if (have_custom_icon()) {
        icon_name = ICON_NAME;
        if (g_strcmp0(state, "active") == 0) {
            status_desc = "P2_app RUNNING";
        } else if (g_strcmp0(state, "failed") == 0) {
            status_desc = "P2_app FAILED";
        }
    } else {
        if (g_strcmp0(state, "active") == 0) {
            icon_name = "media-playback-start";
            status_desc = "P2_app RUNNING";
        } else if (g_strcmp0(state, "failed") == 0) {
            icon_name = "dialog-error";
            status_desc = "P2_app FAILED";
        }
    }

    app_indicator_set_status(ui->indicator, APP_INDICATOR_STATUS_ACTIVE);
    app_indicator_set_icon_full(ui->indicator, icon_name, status_desc);

    if (ui->tray_start_item) {
        gtk_widget_set_sensitive(ui->tray_start_item, !active);
    }
    if (ui->tray_stop_item) {
        gtk_widget_set_sensitive(ui->tray_stop_item, active);
    }
    if (ui->tray_restart_item) {
        gtk_widget_set_sensitive(ui->tray_restart_item, TRUE);
    }
}

static gboolean refresh(gpointer data) {
    UI *ui = (UI *)data;
    char state[128] = {0};
    char enable_state[128] = {0};
    char service_status[160];
    char boot_status[160];
    gboolean active;
    gboolean boot_enabled;

    get_service_state(state, sizeof(state));
    get_enable_state(enable_state, sizeof(enable_state));
    active = is_active_state(state);
    boot_enabled = is_enabled_state(enable_state);

    if (g_strcmp0(state, "failed") == 0) {
        snprintf(service_status, sizeof(service_status), "P2_app: FAILED");
    } else if (active) {
        snprintf(service_status, sizeof(service_status), "P2_app: RUNNING");
    } else {
        snprintf(service_status, sizeof(service_status), "P2_app: %s", *state ? state : "unknown");
    }
    snprintf(boot_status, sizeof(boot_status), "Boot Start: %s", *enable_state ? enable_state : "unknown");

    gtk_label_set_text(GTK_LABEL(ui->status_label), service_status);
    gtk_label_set_text(GTK_LABEL(ui->boot_label), boot_status);
    gtk_widget_set_sensitive(ui->btn_start, !active);
    gtk_widget_set_sensitive(ui->btn_stop, active);
    gtk_widget_set_sensitive(ui->btn_restart, TRUE);
    gtk_widget_set_sensitive(ui->btn_enable_boot, !boot_enabled);
    gtk_widget_set_sensitive(ui->btn_disable_boot, boot_enabled);

    update_tray_state(ui, state, active);
    if (ui->tray_enable_boot_item) {
        gtk_widget_set_sensitive(ui->tray_enable_boot_item, !boot_enabled);
    }
    if (ui->tray_disable_boot_item) {
        gtk_widget_set_sensitive(ui->tray_disable_boot_item, boot_enabled);
    }
    update_window_visibility_item(ui);
    return TRUE;
}

static gboolean create_tray(UI *ui) {
    GtkWidget *tray_quit_item;

    ui->tray_menu = gtk_menu_new();
    ui->tray_show_item = gtk_menu_item_new_with_label("Show Control Window");
    ui->tray_start_item = gtk_menu_item_new_with_label("Start P2_app");
    ui->tray_stop_item = gtk_menu_item_new_with_label("Stop P2_app");
    ui->tray_restart_item = gtk_menu_item_new_with_label("Restart P2_app");
    ui->tray_show_info_item = gtk_menu_item_new_with_label("Show App Info");
    ui->tray_enable_boot_item = gtk_menu_item_new_with_label("Enable at Boot");
    ui->tray_disable_boot_item = gtk_menu_item_new_with_label("Disable at Boot");
    tray_quit_item = gtk_menu_item_new_with_label("Quit");

    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), ui->tray_show_item);
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), gtk_separator_menu_item_new());
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), ui->tray_start_item);
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), ui->tray_stop_item);
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), ui->tray_restart_item);
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), ui->tray_show_info_item);
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), gtk_separator_menu_item_new());
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), ui->tray_enable_boot_item);
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), ui->tray_disable_boot_item);
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), gtk_separator_menu_item_new());
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), tray_quit_item);
    gtk_widget_show_all(ui->tray_menu);

    g_signal_connect(ui->tray_show_item, "activate", G_CALLBACK(on_tray_show_toggle), ui);
    g_signal_connect(ui->tray_start_item, "activate", G_CALLBACK(on_start), ui);
    g_signal_connect(ui->tray_stop_item, "activate", G_CALLBACK(on_stop), ui);
    g_signal_connect(ui->tray_restart_item, "activate", G_CALLBACK(on_restart), ui);
    g_signal_connect(ui->tray_show_info_item, "activate", G_CALLBACK(on_show_info), ui);
    g_signal_connect(ui->tray_enable_boot_item, "activate", G_CALLBACK(on_enable_boot), ui);
    g_signal_connect(ui->tray_disable_boot_item, "activate", G_CALLBACK(on_disable_boot), ui);
    g_signal_connect(tray_quit_item, "activate", G_CALLBACK(on_quit), ui);

    ui->indicator = app_indicator_new_with_path("p2app-control",
                                                have_custom_icon() ? ICON_NAME : "media-playback-stop",
                                                APP_INDICATOR_CATEGORY_SYSTEM_SERVICES,
                                                custom_icon_lookup_dir());
    if (!ui->indicator) {
        g_printerr("Failed to create AppIndicator instance.\n");
        return FALSE;
    }

    app_indicator_set_status(ui->indicator, APP_INDICATOR_STATUS_ACTIVE);
    app_indicator_set_title(ui->indicator, "P2_app Control");
    app_indicator_set_label(ui->indicator, "P2", "P2");
    app_indicator_set_menu(ui->indicator, GTK_MENU(ui->tray_menu));

    return TRUE;
}

static int parse_args(int argc, char **argv, gboolean *tray_mode) {
    int i;

    *tray_mode = FALSE;
    for (i = 1; i < argc; i++) {
        if (g_strcmp0(argv[i], "--tray") == 0) {
            *tray_mode = TRUE;
        } else if (g_strcmp0(argv[i], "--window") == 0) {
            *tray_mode = FALSE;
        } else if (g_strcmp0(argv[i], "-h") == 0 || g_strcmp0(argv[i], "--help") == 0) {
            g_print("Usage: %s [--tray|--window]\n", argv[0]);
            g_print("  --tray    run as panel tray app (AppIndicator)\n");
            g_print("  --window  run as normal control window (default)\n");
            return 0;
        } else {
            g_printerr("Unknown argument: %s\n", argv[i]);
            g_printerr("Try --help\n");
            return -1;
        }
    }
    return 1;
}

int main(int argc, char **argv) {
    UI ui = {0};
    int parse_rc;

    parse_rc = parse_args(argc, argv, &ui.tray_mode);
    if (parse_rc <= 0) {
        return (parse_rc == 0) ? 0 : 1;
    }
    if (parse_rc != 1) {
        return 1;
    }

    gtk_init(&argc, &argv);

    ui.win = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_title(GTK_WINDOW(ui.win), "P2_app Control");
    if (have_custom_icon()) {
        GError *icon_error = NULL;
        gtk_window_set_icon_from_file(GTK_WINDOW(ui.win), custom_icon_file(), &icon_error);
        if (icon_error != NULL) {
            g_error_free(icon_error);
        }
    }
    gtk_window_set_resizable(GTK_WINDOW(ui.win), FALSE);
    gtk_container_set_border_width(GTK_CONTAINER(ui.win), 10);
    gtk_window_set_keep_above(GTK_WINDOW(ui.win), TRUE);

    GtkWidget *vbox = gtk_box_new(GTK_ORIENTATION_VERTICAL, 10);
    GtkWidget *hbox;

    gtk_container_add(GTK_CONTAINER(ui.win), vbox);

    ui.status_label = gtk_label_new("P2_app: ...");
    ui.boot_label = gtk_label_new("Boot Start: ...");
    gtk_label_set_xalign(GTK_LABEL(ui.status_label), 0.0f);
    gtk_label_set_xalign(GTK_LABEL(ui.boot_label), 0.0f);
    gtk_box_pack_start(GTK_BOX(vbox), ui.status_label, FALSE, FALSE, 0);
    gtk_box_pack_start(GTK_BOX(vbox), ui.boot_label, FALSE, FALSE, 0);

    hbox = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_box_pack_start(GTK_BOX(vbox), hbox, FALSE, FALSE, 0);

    ui.btn_start = gtk_button_new_with_label("Start");
    ui.btn_stop = gtk_button_new_with_label("Stop");
    ui.btn_restart = gtk_button_new_with_label("Restart");
    gtk_box_pack_start(GTK_BOX(hbox), ui.btn_start, TRUE, TRUE, 0);
    gtk_box_pack_start(GTK_BOX(hbox), ui.btn_stop, TRUE, TRUE, 0);
    gtk_box_pack_start(GTK_BOX(hbox), ui.btn_restart, TRUE, TRUE, 0);

    hbox = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_box_pack_start(GTK_BOX(vbox), hbox, FALSE, FALSE, 0);

    ui.btn_enable_boot = gtk_button_new_with_label("Enable Boot");
    ui.btn_disable_boot = gtk_button_new_with_label("Disable Boot");
    gtk_box_pack_start(GTK_BOX(hbox), ui.btn_enable_boot, TRUE, TRUE, 0);
    gtk_box_pack_start(GTK_BOX(hbox), ui.btn_disable_boot, TRUE, TRUE, 0);

    hbox = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_box_pack_start(GTK_BOX(vbox), hbox, FALSE, FALSE, 0);

    ui.btn_show_info = gtk_button_new_with_label("App Info");
    ui.btn_quit = gtk_button_new_with_label("Quit");
    gtk_box_pack_start(GTK_BOX(hbox), ui.btn_show_info, TRUE, TRUE, 0);
    gtk_box_pack_start(GTK_BOX(hbox), ui.btn_quit, TRUE, TRUE, 0);

    g_signal_connect(ui.btn_start, "clicked", G_CALLBACK(on_start), &ui);
    g_signal_connect(ui.btn_stop, "clicked", G_CALLBACK(on_stop), &ui);
    g_signal_connect(ui.btn_restart, "clicked", G_CALLBACK(on_restart), &ui);
    g_signal_connect(ui.btn_enable_boot, "clicked", G_CALLBACK(on_enable_boot), &ui);
    g_signal_connect(ui.btn_disable_boot, "clicked", G_CALLBACK(on_disable_boot), &ui);
    g_signal_connect(ui.btn_show_info, "clicked", G_CALLBACK(on_show_info), &ui);
    g_signal_connect(ui.btn_quit, "clicked", G_CALLBACK(on_quit), &ui);
    g_signal_connect(ui.win, "delete-event", G_CALLBACK(on_window_delete), &ui);
    g_signal_connect(ui.win, "destroy", G_CALLBACK(on_window_destroy), &ui);

    if (ui.tray_mode) {
        if (create_tray(&ui)) {
            gtk_widget_show_all(ui.win);
            hide_window(&ui);
        } else {
            g_printerr("Tray mode unavailable; falling back to window mode.\n");
            ui.tray_mode = FALSE;
            gtk_widget_show_all(ui.win);
        }
    } else {
        gtk_widget_show_all(ui.win);
    }

    g_timeout_add(1000, refresh, &ui);
    refresh(&ui);
    gtk_main();
    return 0;
}
