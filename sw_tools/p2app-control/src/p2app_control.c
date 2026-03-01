#include <gtk/gtk.h>
#include <libayatana-appindicator/app-indicator.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *UNIT = "p2app.service";

typedef struct {
    gboolean tray_mode;
    GtkWidget *win;
    GtkWidget *label;
    GtkWidget *btn_start;
    GtkWidget *btn_stop;
    GtkWidget *btn_restart;
    GtkWidget *btn_quit;
    AppIndicator *indicator;
    GtkWidget *tray_menu;
    GtkWidget *tray_show_item;
    GtkWidget *tray_start_item;
    GtkWidget *tray_stop_item;
    GtkWidget *tray_restart_item;
} UI;

static gboolean run_capture(const char *cmd, char *out, gsize outlen) {
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

    if (out && outlen) {
        const char *src = (stdout_buf && *stdout_buf) ? stdout_buf : (stderr_buf ? stderr_buf : "");
        g_strlcpy(out, src, outlen);
    }

    g_free(stdout_buf);
    g_free(stderr_buf);
    return TRUE;
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

static gboolean is_active_state(const char *state) {
    return g_strcmp0(state, "active") == 0;
}

static void pkexec_systemctl(const char *verb) {
    char cmd[256];
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
    pkexec_systemctl("start");
}

static void on_stop(GtkWidget *unused, gpointer data) {
    (void)unused;
    (void)data;
    pkexec_systemctl("stop");
}

static void on_restart(GtkWidget *unused, gpointer data) {
    (void)unused;
    (void)data;
    pkexec_systemctl("restart");
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

    if (g_strcmp0(state, "active") == 0) {
        icon_name = "media-playback-start";
        status_desc = "P2_app RUNNING";
    } else if (g_strcmp0(state, "failed") == 0) {
        icon_name = "dialog-error";
        status_desc = "P2_app FAILED";
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
    char window_status[160];
    gboolean active;

    get_service_state(state, sizeof(state));
    active = is_active_state(state);

    snprintf(window_status, sizeof(window_status), "P2_app: %s", active ? "RUNNING" : "STOPPED");
    gtk_label_set_text(GTK_LABEL(ui->label), window_status);
    gtk_widget_set_sensitive(ui->btn_start, !active);
    gtk_widget_set_sensitive(ui->btn_stop, active);
    gtk_widget_set_sensitive(ui->btn_restart, TRUE);

    update_tray_state(ui, state, active);
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
    tray_quit_item = gtk_menu_item_new_with_label("Quit");

    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), ui->tray_show_item);
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), gtk_separator_menu_item_new());
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), ui->tray_start_item);
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), ui->tray_stop_item);
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), ui->tray_restart_item);
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), gtk_separator_menu_item_new());
    gtk_menu_shell_append(GTK_MENU_SHELL(ui->tray_menu), tray_quit_item);
    gtk_widget_show_all(ui->tray_menu);

    g_signal_connect(ui->tray_show_item, "activate", G_CALLBACK(on_tray_show_toggle), ui);
    g_signal_connect(ui->tray_start_item, "activate", G_CALLBACK(on_start), ui);
    g_signal_connect(ui->tray_stop_item, "activate", G_CALLBACK(on_stop), ui);
    g_signal_connect(ui->tray_restart_item, "activate", G_CALLBACK(on_restart), ui);
    g_signal_connect(tray_quit_item, "activate", G_CALLBACK(on_quit), ui);

    ui->indicator = app_indicator_new_with_path("p2app-control",
                                                "media-playback-stop",
                                                APP_INDICATOR_CATEGORY_SYSTEM_SERVICES,
                                                NULL);
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
    gtk_window_set_resizable(GTK_WINDOW(ui.win), FALSE);
    gtk_container_set_border_width(GTK_CONTAINER(ui.win), 10);
    gtk_window_set_keep_above(GTK_WINDOW(ui.win), TRUE);

    GtkWidget *vbox = gtk_box_new(GTK_ORIENTATION_VERTICAL, 10);
    GtkWidget *hbox;

    gtk_container_add(GTK_CONTAINER(ui.win), vbox);

    ui.label = gtk_label_new("P2_app: ...");
    gtk_box_pack_start(GTK_BOX(vbox), ui.label, FALSE, FALSE, 0);

    hbox = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_box_pack_start(GTK_BOX(vbox), hbox, FALSE, FALSE, 0);

    ui.btn_start = gtk_button_new_with_label("Start");
    ui.btn_stop = gtk_button_new_with_label("Stop");
    ui.btn_restart = gtk_button_new_with_label("Restart");
    gtk_box_pack_start(GTK_BOX(hbox), ui.btn_start, TRUE, TRUE, 0);
    gtk_box_pack_start(GTK_BOX(hbox), ui.btn_stop, TRUE, TRUE, 0);
    gtk_box_pack_start(GTK_BOX(hbox), ui.btn_restart, TRUE, TRUE, 0);

    ui.btn_quit = gtk_button_new_with_label("Quit");
    gtk_box_pack_start(GTK_BOX(vbox), ui.btn_quit, FALSE, FALSE, 0);

    g_signal_connect(ui.btn_start, "clicked", G_CALLBACK(on_start), &ui);
    g_signal_connect(ui.btn_stop, "clicked", G_CALLBACK(on_stop), &ui);
    g_signal_connect(ui.btn_restart, "clicked", G_CALLBACK(on_restart), &ui);
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
