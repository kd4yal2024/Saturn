#include <gtk/gtk.h>
#include <glib/gstdio.h>
#include <sys/stat.h>

#include <cerrno>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

struct UiState
{
    GtkWidget *window;
    GtkWidget *status_label;
    GtkWidget *result_label;
    GtkWidget *timer_label;
    GtkWidget *progress_bar;
    GtkWidget *log_frame;
    GtkWidget *log_scroller;
    GtkWidget *log_view;
    GtkWidget *toggle_button;
    GtkWidget *install_button;
    GtkWidget *close_button;
    GtkTextBuffer *log_buffer;

    std::string log_file;
    std::string status_file;
    std::string runner_path;
    std::string desktop_shortcut;
    std::string window_title;

    gint64 install_started_us;
    gsize log_offset;
    GPid runner_pid;
    bool install_running;
    bool install_succeeded;
};

static std::string trim_copy(const std::string &value)
{
    const std::string whitespace = " \t\r\n";
    const auto begin = value.find_first_not_of(whitespace);
    if (begin == std::string::npos)
    {
        return "";
    }
    const auto end = value.find_last_not_of(whitespace);
    return value.substr(begin, end - begin + 1);
}

static std::string format_duration(guint64 total_seconds)
{
    const guint64 hours = total_seconds / 3600;
    const guint64 minutes = (total_seconds % 3600) / 60;
    const guint64 seconds = total_seconds % 60;
    char buffer[32];
    g_snprintf(buffer, sizeof(buffer), "%02llu:%02llu:%02llu",
               static_cast<unsigned long long>(hours),
               static_cast<unsigned long long>(minutes),
               static_cast<unsigned long long>(seconds));
    return std::string(buffer);
}

static bool read_status_line(const std::string &path, std::string *state, std::string *message)
{
    std::ifstream in(path);
    if (!in)
    {
        return false;
    }

    std::string line;
    if (!std::getline(in, line) || line.empty())
    {
        return false;
    }

    std::stringstream ss(line);
    std::string token;
    std::string fields[3];
    int index = 0;
    while (std::getline(ss, token, '|') && index < 3)
    {
        fields[index++] = trim_copy(token);
    }
    if (index < 1)
    {
        return false;
    }

    *state = fields[0];
    if (index >= 3)
    {
        *message = fields[2];
    }
    else if (index == 2)
    {
        *message = fields[1];
    }
    else
    {
        message->clear();
    }
    return true;
}

static void append_log_delta(UiState *ui)
{
    struct stat st;
    if (stat(ui->log_file.c_str(), &st) != 0)
    {
        return;
    }

    if (static_cast<gsize>(st.st_size) < ui->log_offset)
    {
        ui->log_offset = 0;
        gtk_text_buffer_set_text(ui->log_buffer, "", -1);
    }

    std::ifstream in(ui->log_file, std::ios::binary);
    if (!in)
    {
        return;
    }

    in.seekg(static_cast<std::streamoff>(ui->log_offset), std::ios::beg);
    std::string chunk((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
    if (chunk.empty())
    {
        return;
    }

    ui->log_offset += chunk.size();

    GtkTextIter end_iter;
    gtk_text_buffer_get_end_iter(ui->log_buffer, &end_iter);
    gtk_text_buffer_insert(ui->log_buffer, &end_iter, chunk.c_str(), -1);

    gtk_text_buffer_get_end_iter(ui->log_buffer, &end_iter);
    gtk_text_view_scroll_to_iter(GTK_TEXT_VIEW(ui->log_view), &end_iter, 0.0, FALSE, 0.0, 1.0);
}

static void set_result(UiState *ui, const char *markup)
{
    gtk_label_set_markup(GTK_LABEL(ui->result_label), markup);
}

static void set_status(UiState *ui, const std::string &message)
{
    gtk_label_set_text(GTK_LABEL(ui->status_label), message.c_str());
}

static void set_install_button(UiState *ui, const char *label, gboolean sensitive)
{
    gtk_button_set_label(GTK_BUTTON(ui->install_button), label);
    gtk_widget_set_sensitive(ui->install_button, sensitive);
}

static void maybe_remove_shortcut(UiState *ui)
{
    if (!ui->install_succeeded || ui->desktop_shortcut.empty())
    {
        return;
    }

    if (g_remove(ui->desktop_shortcut.c_str()) == 0 || errno == ENOENT)
    {
        return;
    }
}

static void on_runner_exit(GPid pid, gint, gpointer user_data)
{
    UiState *ui = static_cast<UiState *>(user_data);
    if (ui->runner_pid == pid)
    {
        ui->runner_pid = 0;
    }
    ui->install_running = false;
    g_spawn_close_pid(pid);
}

static bool spawn_runner(UiState *ui, std::string *error_text)
{
    gchar *argv[] = {
        const_cast<gchar *>(ui->runner_path.c_str()),
        const_cast<gchar *>("--status-file"),
        const_cast<gchar *>(ui->status_file.c_str()),
        const_cast<gchar *>("--log-file"),
        const_cast<gchar *>(ui->log_file.c_str()),
        nullptr};

    GError *error = nullptr;
    GPid pid = 0;
    if (!g_spawn_async(nullptr,
                       argv,
                       nullptr,
                       static_cast<GSpawnFlags>(G_SPAWN_SEARCH_PATH | G_SPAWN_DO_NOT_REAP_CHILD),
                       nullptr,
                       nullptr,
                       &pid,
                       &error))
    {
        *error_text = error && error->message ? error->message : "Failed to start installer.";
        if (error)
        {
            g_error_free(error);
        }
        return false;
    }

    ui->runner_pid = pid;
    g_child_watch_add(pid, on_runner_exit, ui);
    return true;
}

static void on_install_clicked(GtkButton *, gpointer user_data)
{
    UiState *ui = static_cast<UiState *>(user_data);
    if (ui->install_running)
    {
        return;
    }

    if (ui->runner_path.empty())
    {
        set_result(ui, "<span foreground='#ff7f7f' weight='bold' size='x-large'>Installer unavailable</span>");
        set_status(ui, "Standalone installer runner path is missing.");
        return;
    }

    ui->install_running = true;
    ui->install_succeeded = false;
    ui->install_started_us = g_get_monotonic_time();
    ui->log_offset = 0;
    gtk_text_buffer_set_text(ui->log_buffer, "", -1);
    gtk_progress_bar_set_fraction(GTK_PROGRESS_BAR(ui->progress_bar), 0.0);
    gtk_progress_bar_set_text(GTK_PROGRESS_BAR(ui->progress_bar), "Installing...");
    set_result(ui, "<span foreground='#eaf3ff' weight='bold' size='x-large'>Installing piHPSDR...</span>");
    set_status(ui, "Launching the standalone piHPSDR installer.");
    set_install_button(ui, "Installing...", FALSE);

    std::string error_text;
    if (!spawn_runner(ui, &error_text))
    {
        ui->install_running = false;
        gtk_progress_bar_set_fraction(GTK_PROGRESS_BAR(ui->progress_bar), 1.0);
        gtk_progress_bar_set_text(GTK_PROGRESS_BAR(ui->progress_bar), "Failed");
        set_result(ui, "<span foreground='#ff7f7f' weight='bold' size='x-large'>Installer failed to start</span>");
        set_status(ui, error_text.empty() ? "Failed to start the standalone installer." : error_text);
        set_install_button(ui, "Retry Install", TRUE);
    }
}

static void close_ui(UiState *ui)
{
    maybe_remove_shortcut(ui);
    gtk_main_quit();
}

static gboolean handle_close_request(UiState *ui)
{
    if (ui->install_running)
    {
        GtkWidget *dialog = gtk_message_dialog_new(
            GTK_WINDOW(ui->window),
            static_cast<GtkDialogFlags>(GTK_DIALOG_MODAL | GTK_DIALOG_DESTROY_WITH_PARENT),
            GTK_MESSAGE_QUESTION,
            GTK_BUTTONS_NONE,
            "%s",
            "piHPSDR installation is still running.");
        gtk_message_dialog_format_secondary_text(
            GTK_MESSAGE_DIALOG(dialog),
            "%s",
            "You can close this window now and reopen it later from the Desktop shortcut to continue watching progress.");
        gtk_dialog_add_button(GTK_DIALOG(dialog), "Keep Open", GTK_RESPONSE_CANCEL);
        gtk_dialog_add_button(GTK_DIALOG(dialog), "Close Window", GTK_RESPONSE_ACCEPT);
        gtk_dialog_set_default_response(GTK_DIALOG(dialog), GTK_RESPONSE_CANCEL);

        const gint response = gtk_dialog_run(GTK_DIALOG(dialog));
        gtk_widget_destroy(dialog);
        if (response != GTK_RESPONSE_ACCEPT)
        {
            return FALSE;
        }
    }

    close_ui(ui);
    return TRUE;
}

static void on_close_clicked(GtkButton *, gpointer user_data)
{
    UiState *ui = static_cast<UiState *>(user_data);
    handle_close_request(ui);
}

static gboolean on_window_delete(GtkWidget *, GdkEvent *, gpointer user_data)
{
    UiState *ui = static_cast<UiState *>(user_data);
    if (!handle_close_request(ui))
    {
        return TRUE;
    }
    return TRUE;
}

static gboolean on_tick(gpointer user_data)
{
    UiState *ui = static_cast<UiState *>(user_data);
    append_log_delta(ui);

    const gint64 now_us = g_get_monotonic_time();
    if (ui->install_started_us > 0)
    {
        const guint64 elapsed = static_cast<guint64>((now_us - ui->install_started_us) / G_USEC_PER_SEC);
        gtk_label_set_text(GTK_LABEL(ui->timer_label), ("Elapsed: " + format_duration(elapsed)).c_str());
    }
    else
    {
        gtk_label_set_text(GTK_LABEL(ui->timer_label), "Ready to install");
    }

    std::string status_state;
    std::string status_message;
    const bool has_status = read_status_line(ui->status_file, &status_state, &status_message);

    if (has_status && status_state == "RUNNING")
    {
        if (ui->install_started_us == 0)
        {
            ui->install_started_us = now_us;
        }
        ui->install_running = true;
        ui->install_succeeded = false;
        gtk_progress_bar_pulse(GTK_PROGRESS_BAR(ui->progress_bar));
        gtk_progress_bar_set_text(GTK_PROGRESS_BAR(ui->progress_bar), "Installing...");
        set_result(ui, "<span foreground='#eaf3ff' weight='bold' size='x-large'>Installing piHPSDR...</span>");
        set_status(ui, status_message.empty() ? "piHPSDR install is running." : status_message);
        set_install_button(ui, "Installing...", FALSE);
    }
    else if (has_status && status_state == "SUCCESS")
    {
        ui->install_running = false;
        ui->install_succeeded = true;
        gtk_progress_bar_set_fraction(GTK_PROGRESS_BAR(ui->progress_bar), 1.0);
        gtk_progress_bar_set_text(GTK_PROGRESS_BAR(ui->progress_bar), "Done");
        set_result(ui, "<span foreground='#8bf58b' weight='bold' size='x-large'>piHPSDR install complete</span>");
        if (status_message.empty())
        {
            set_status(ui, "piHPSDR is ready. Click Close to remove the installer shortcut from the Desktop.");
        }
        else
        {
            set_status(ui, status_message + " Click Close to remove the installer shortcut from the Desktop.");
        }
        set_install_button(ui, "Installed", FALSE);
    }
    else if (has_status && status_state == "FAILED")
    {
        ui->install_running = false;
        ui->install_succeeded = false;
        gtk_progress_bar_set_fraction(GTK_PROGRESS_BAR(ui->progress_bar), 1.0);
        gtk_progress_bar_set_text(GTK_PROGRESS_BAR(ui->progress_bar), "Failed");
        set_result(ui, "<span foreground='#ff7f7f' weight='bold' size='x-large'>piHPSDR install failed</span>");
        set_status(ui, status_message.empty() ? "The installer reported a failure. Review the terminal output below." : status_message);
        set_install_button(ui, "Retry Install", TRUE);
    }
    else if (!ui->install_running && !ui->install_succeeded)
    {
        gtk_progress_bar_set_fraction(GTK_PROGRESS_BAR(ui->progress_bar), 0.0);
        gtk_progress_bar_set_text(GTK_PROGRESS_BAR(ui->progress_bar), "Ready");
        set_result(ui, "<span foreground='#eaf3ff' weight='bold' size='x-large'>Ready to install piHPSDR</span>");
        set_status(ui, "Click Install to clone or update piHPSDR and build it on this system.");
        set_install_button(ui, "Install piHPSDR", TRUE);
    }

    return G_SOURCE_CONTINUE;
}

static void on_toggle_log(GtkToggleButton *button, gpointer user_data)
{
    UiState *ui = static_cast<UiState *>(user_data);
    const gboolean show = gtk_toggle_button_get_active(button);
    gtk_widget_set_visible(ui->log_frame, show);
    gtk_button_set_label(GTK_BUTTON(ui->toggle_button), show ? "Hide Terminal Output" : "Show Terminal Output");
}

int main(int argc, char **argv)
{
    gchar *arg_log = g_strdup("");
    gchar *arg_status = g_strdup("");
    gchar *arg_runner = g_strdup("/usr/local/bin/pihpsdr-installer-run.sh");
    gchar *arg_shortcut = g_strdup("");
    gchar *arg_window_title = g_strdup("piHPSDR Installer");
    gchar *arg_icon_file = nullptr;
    gboolean arg_show_log = FALSE;

    GOptionEntry entries[] = {
        {"log-file", 0, 0, G_OPTION_ARG_FILENAME, &arg_log, "Installer log file", "PATH"},
        {"status-file", 0, 0, G_OPTION_ARG_FILENAME, &arg_status, "Installer status file", "PATH"},
        {"runner", 0, 0, G_OPTION_ARG_FILENAME, &arg_runner, "Runner script path", "PATH"},
        {"desktop-shortcut", 0, 0, G_OPTION_ARG_FILENAME, &arg_shortcut, "Desktop shortcut path to remove after successful close", "PATH"},
        {"window-title", 0, 0, G_OPTION_ARG_STRING, &arg_window_title, "Installer window title", "TEXT"},
        {"icon-file", 0, 0, G_OPTION_ARG_FILENAME, &arg_icon_file, "Window/launcher icon file", "PATH"},
        {"show-log", 0, 0, G_OPTION_ARG_NONE, &arg_show_log, "Show terminal output panel on startup", nullptr},
        {nullptr, 0, 0, G_OPTION_ARG_NONE, nullptr, nullptr, nullptr}};

    GOptionContext *context = g_option_context_new("- piHPSDR Installer UI");
    g_option_context_add_main_entries(context, entries, nullptr);
    g_option_context_add_group(context, gtk_get_option_group(TRUE));

    GError *error = nullptr;
    if (!g_option_context_parse(context, &argc, &argv, &error))
    {
        g_printerr("Failed to parse arguments: %s\n", error ? error->message : "unknown error");
        if (error)
        {
            g_error_free(error);
        }
        g_option_context_free(context);
        g_free(arg_log);
        g_free(arg_status);
        g_free(arg_runner);
        g_free(arg_shortcut);
        g_free(arg_window_title);
        return 1;
    }
    g_option_context_free(context);

    UiState ui{};
    ui.log_file = arg_log ? arg_log : "";
    ui.status_file = arg_status ? arg_status : "";
    ui.runner_path = arg_runner ? arg_runner : "";
    ui.desktop_shortcut = arg_shortcut ? arg_shortcut : "";
    ui.window_title = arg_window_title ? arg_window_title : "piHPSDR Installer";
    ui.install_started_us = 0;
    ui.log_offset = 0;
    ui.runner_pid = 0;
    ui.install_running = false;
    ui.install_succeeded = false;

    g_free(arg_log);
    g_free(arg_status);
    g_free(arg_runner);
    g_free(arg_shortcut);
    g_free(arg_window_title);

    ui.window = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_title(GTK_WINDOW(ui.window), ui.window_title.c_str());
    gtk_window_set_default_size(GTK_WINDOW(ui.window), 920, 620);
    gtk_widget_set_name(ui.window, "installer_window");
    gtk_window_set_position(GTK_WINDOW(ui.window), GTK_WIN_POS_CENTER);
    if (arg_icon_file && *arg_icon_file)
    {
        GError *icon_error = nullptr;
        gtk_window_set_icon_from_file(GTK_WINDOW(ui.window), arg_icon_file, &icon_error);
        if (icon_error)
        {
            g_error_free(icon_error);
        }
    }

    GtkCssProvider *css_provider = gtk_css_provider_new();
    gtk_css_provider_load_from_data(
        css_provider,
        "#installer_window {"
        "  background-image: linear-gradient(135deg, #07151f 0%, #12314d 48%, #14555f 100%);"
        "}"
        "#hero_card {"
        "  background: rgba(255, 255, 255, 0.08);"
        "  border-radius: 18px;"
        "  padding: 18px;"
        "}"
        "#title_label {"
        "  color: #f7fbff;"
        "  font-size: 30px;"
        "  font-weight: 700;"
        "}"
        "#subtitle_label {"
        "  color: #b8d8f7;"
        "  font-size: 14px;"
        "}"
        "#result_label {"
        "  color: #ecf7ff;"
        "}"
        "#status_label {"
        "  color: #cfe7ff;"
        "  font-size: 15px;"
        "}"
        "#timer_label {"
        "  color: #9dc7f5;"
        "  font-size: 13px;"
        "}"
        "#log_frame {"
        "  border-radius: 14px;"
        "  background: rgba(6, 15, 24, 0.48);"
        "}"
        "#log_frame > border {"
        "  border-radius: 14px;"
        "  border: 1px solid rgba(180, 220, 255, 0.16);"
        "}"
        "button {"
        "  border-radius: 999px;"
        "  padding: 8px 16px;"
        "}"
        "progressbar trough {"
        "  min-height: 16px;"
        "  border-radius: 999px;"
        "  background: rgba(255,255,255,0.14);"
        "}"
        "progressbar progress {"
        "  border-radius: 999px;"
        "  background-image: linear-gradient(90deg, #70d6ff 0%, #8ef4c5 100%);"
        "}"
        "textview, textview text {"
        "  background: rgba(6, 12, 18, 0.92);"
        "  color: #d8f2ff;"
        "}"
        "textview {"
        "  font-family: Monospace;"
        "  font-size: 10.5pt;"
        "}",
        -1,
        nullptr);
    gtk_style_context_add_provider_for_screen(gdk_screen_get_default(),
                                              GTK_STYLE_PROVIDER(css_provider),
                                              GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
    g_object_unref(css_provider);

    GtkWidget *root = gtk_box_new(GTK_ORIENTATION_VERTICAL, 10);
    gtk_container_set_border_width(GTK_CONTAINER(root), 18);
    gtk_container_add(GTK_CONTAINER(ui.window), root);

    GtkWidget *hero_card = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 16);
    gtk_widget_set_name(hero_card, "hero_card");
    gtk_box_pack_start(GTK_BOX(root), hero_card, FALSE, FALSE, 0);

    if (arg_icon_file && *arg_icon_file)
    {
        GError *pixbuf_error = nullptr;
        GdkPixbuf *pixbuf = gdk_pixbuf_new_from_file_at_scale(arg_icon_file, 88, 88, TRUE, &pixbuf_error);
        if (pixbuf != nullptr)
        {
            GtkWidget *icon_image = gtk_image_new_from_pixbuf(pixbuf);
            gtk_box_pack_start(GTK_BOX(hero_card), icon_image, FALSE, FALSE, 0);
            g_object_unref(pixbuf);
        }
        if (pixbuf_error != nullptr)
        {
            g_error_free(pixbuf_error);
        }
    }

    GtkWidget *hero_text = gtk_box_new(GTK_ORIENTATION_VERTICAL, 6);
    gtk_box_pack_start(GTK_BOX(hero_card), hero_text, TRUE, TRUE, 0);

    GtkWidget *title_label = gtk_label_new(ui.window_title.c_str());
    gtk_widget_set_name(title_label, "title_label");
    gtk_widget_set_halign(title_label, GTK_ALIGN_START);
    gtk_box_pack_start(GTK_BOX(hero_text), title_label, FALSE, FALSE, 0);

    GtkWidget *subtitle_label = gtk_label_new("Standalone installer for piHPSDR with live terminal output. Click Install to clone or update the app and build it locally.");
    gtk_widget_set_name(subtitle_label, "subtitle_label");
    gtk_widget_set_halign(subtitle_label, GTK_ALIGN_START);
    gtk_label_set_line_wrap(GTK_LABEL(subtitle_label), TRUE);
    gtk_box_pack_start(GTK_BOX(hero_text), subtitle_label, FALSE, FALSE, 0);

    ui.result_label = gtk_label_new(nullptr);
    gtk_widget_set_name(ui.result_label, "result_label");
    gtk_widget_set_halign(ui.result_label, GTK_ALIGN_START);
    gtk_label_set_markup(GTK_LABEL(ui.result_label),
                         "<span foreground='#eaf3ff' weight='bold' size='x-large'>Ready to install piHPSDR</span>");
    gtk_box_pack_start(GTK_BOX(root), ui.result_label, FALSE, FALSE, 0);

    ui.status_label = gtk_label_new("Click Install to start the standalone piHPSDR installer.");
    gtk_widget_set_name(ui.status_label, "status_label");
    gtk_widget_set_halign(ui.status_label, GTK_ALIGN_START);
    gtk_label_set_line_wrap(GTK_LABEL(ui.status_label), TRUE);
    gtk_box_pack_start(GTK_BOX(root), ui.status_label, FALSE, FALSE, 0);

    ui.timer_label = gtk_label_new("Ready to install");
    gtk_widget_set_name(ui.timer_label, "timer_label");
    gtk_widget_set_halign(ui.timer_label, GTK_ALIGN_START);
    gtk_box_pack_start(GTK_BOX(root), ui.timer_label, FALSE, FALSE, 0);

    ui.progress_bar = gtk_progress_bar_new();
    gtk_progress_bar_set_show_text(GTK_PROGRESS_BAR(ui.progress_bar), TRUE);
    gtk_progress_bar_set_text(GTK_PROGRESS_BAR(ui.progress_bar), "Ready");
    gtk_box_pack_start(GTK_BOX(root), ui.progress_bar, FALSE, FALSE, 0);

    GtkWidget *button_row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_box_pack_start(GTK_BOX(root), button_row, FALSE, FALSE, 2);

    ui.install_button = gtk_button_new_with_label("Install piHPSDR");
    gtk_box_pack_start(GTK_BOX(button_row), ui.install_button, FALSE, FALSE, 0);

    ui.toggle_button = gtk_toggle_button_new_with_label("Show Terminal Output");
    gtk_box_pack_start(GTK_BOX(button_row), ui.toggle_button, FALSE, FALSE, 0);

    ui.close_button = gtk_button_new_with_label("Close");
    gtk_box_pack_end(GTK_BOX(button_row), ui.close_button, FALSE, FALSE, 0);

    ui.log_frame = gtk_frame_new("Terminal Output");
    gtk_widget_set_name(ui.log_frame, "log_frame");
    gtk_widget_set_vexpand(ui.log_frame, TRUE);
    gtk_widget_set_hexpand(ui.log_frame, TRUE);
    gtk_box_pack_start(GTK_BOX(root), ui.log_frame, TRUE, TRUE, 0);

    ui.log_scroller = gtk_scrolled_window_new(nullptr, nullptr);
    gtk_widget_set_vexpand(ui.log_scroller, TRUE);
    gtk_widget_set_hexpand(ui.log_scroller, TRUE);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(ui.log_scroller), GTK_POLICY_AUTOMATIC, GTK_POLICY_AUTOMATIC);
    gtk_container_add(GTK_CONTAINER(ui.log_frame), ui.log_scroller);

    ui.log_view = gtk_text_view_new();
    gtk_text_view_set_editable(GTK_TEXT_VIEW(ui.log_view), FALSE);
    gtk_text_view_set_cursor_visible(GTK_TEXT_VIEW(ui.log_view), FALSE);
    gtk_text_view_set_wrap_mode(GTK_TEXT_VIEW(ui.log_view), GTK_WRAP_WORD_CHAR);
    gtk_container_add(GTK_CONTAINER(ui.log_scroller), ui.log_view);
    ui.log_buffer = gtk_text_view_get_buffer(GTK_TEXT_VIEW(ui.log_view));

    g_signal_connect(ui.install_button, "clicked", G_CALLBACK(on_install_clicked), &ui);
    g_signal_connect(ui.toggle_button, "toggled", G_CALLBACK(on_toggle_log), &ui);
    g_signal_connect(ui.close_button, "clicked", G_CALLBACK(on_close_clicked), &ui);
    g_signal_connect(ui.window, "delete-event", G_CALLBACK(on_window_delete), &ui);
    g_signal_connect_swapped(ui.window, "destroy", G_CALLBACK(gtk_main_quit), nullptr);

    gtk_toggle_button_set_active(GTK_TOGGLE_BUTTON(ui.toggle_button), arg_show_log ? TRUE : FALSE);

    g_free(arg_icon_file);

    on_tick(&ui);
    g_timeout_add_seconds(1, on_tick, &ui);
    gtk_widget_show_all(ui.window);
    if (!arg_show_log)
    {
        gtk_widget_set_visible(ui.log_frame, FALSE);
    }
    gtk_main();
    return 0;
}
