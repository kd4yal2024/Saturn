#include <gtk/gtk.h>
#include <sys/stat.h>

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
    GtkWidget *log_scroller;
    GtkWidget *log_view;
    GtkWidget *toggle_button;
    GtkWidget *reboot_button;
    GtkWidget *close_button;
    GtkTextBuffer *log_buffer;

    std::string log_file;
    std::string completion_file;
    std::string status_file;

    guint timeout_seconds;
    gint64 start_us;
    gsize log_offset;
    bool finished;
    bool reboot_prompt_shown;
};

static bool file_exists(const std::string &path)
{
    struct stat st;
    return stat(path.c_str(), &st) == 0;
}

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

static int provisioning_stage_index(const std::string &message)
{
    static const std::vector<std::string> stages = {
        "Resolving Saturn user account",
        "Installing desktop provisioning UI prerequisites",
        "Installing desktop power helper",
        "Preparing desktop provisioning UI autostart",
        "Launching desktop provisioning interface",
        "Installing build/runtime dependencies",
        "Configuring USB boot and input tuning",
        "Configuring I2C, SSH, and VNC",
        "Detecting front panel",
        "Installing udev rules",
        "Resolving hardware role",
        "Applying LCD boot profile",
        "Installing developer desktop tools",
        "Syncing Saturn repository",
        "Enabling Python repo guard",
        "Preparing Python virtual environment",
        "Building Saturn applications and tools",
        "Installing desktop launchers",
        "Installing shutdown waiter service",
        "Configuring power-switch LED",
        "Building and installing XDMA module",
        "Installing p2app-control service",
        "Installing Saturn update manager",
        "Installing piHPSDR DSP dependencies",
        "Installing Saturn Remote bridge",
        "Installing standalone piHPSDR shortcut",
        "Flashing FPGA image",
        "Finalizing provisioning state",
        "Cleaning temporary files",
    };

    for (gsize index = 0; index < stages.size(); ++index)
    {
        if (message == stages[index])
        {
            return static_cast<int>(index);
        }
    }
    return -1;
}

static bool try_command_sync(gchar **argv, std::string *error_text)
{
    GError *error = nullptr;
    gchar *standard_output = nullptr;
    gchar *standard_error = nullptr;
    gint wait_status = 0;

    const gboolean spawned = g_spawn_sync(nullptr,
                                          argv,
                                          nullptr,
                                          static_cast<GSpawnFlags>(G_SPAWN_SEARCH_PATH),
                                          nullptr,
                                          nullptr,
                                          &standard_output,
                                          &standard_error,
                                          &wait_status,
                                          &error);
    if (!spawned)
    {
        if (error)
        {
            *error_text = error->message ? error->message : "Failed to start command.";
            g_error_free(error);
        }
        else
        {
            *error_text = "Failed to start command.";
        }
        g_free(standard_output);
        g_free(standard_error);
        return false;
    }

    if (g_spawn_check_wait_status(wait_status, &error))
    {
        g_free(standard_output);
        g_free(standard_error);
        return true;
    }

    std::string combined_error;
    if (error && error->message)
    {
        combined_error = error->message;
        g_error_free(error);
    }
    if (standard_error && *standard_error)
    {
        if (!combined_error.empty())
        {
            combined_error += " ";
        }
        combined_error += standard_error;
    }
    if (combined_error.empty())
    {
        combined_error = "Command exited unsuccessfully.";
    }
    *error_text = trim_copy(combined_error);
    g_free(standard_output);
    g_free(standard_error);
    return false;
}

static void request_reboot(UiState *ui)
{
    std::vector<std::vector<gchar *>> commands = {
        {const_cast<gchar *>("/usr/bin/sudo"), const_cast<gchar *>("-n"), const_cast<gchar *>("/usr/local/sbin/saturn-provision-powerctl"), const_cast<gchar *>("reboot"), nullptr},
        {const_cast<gchar *>("pkexec"), const_cast<gchar *>("/usr/local/sbin/saturn-provision-powerctl"), const_cast<gchar *>("reboot"), nullptr},
        {const_cast<gchar *>("/usr/bin/systemctl"), const_cast<gchar *>("reboot"), nullptr},
    };

    gtk_widget_set_sensitive(ui->reboot_button, FALSE);
    set_status(ui, "Requesting reboot...");

    std::string last_error;
    for (auto &command : commands)
    {
        if (try_command_sync(command.data(), &last_error))
        {
            set_status(ui, "Reboot requested. The system should restart shortly.");
            return;
        }
    }

    gtk_widget_set_sensitive(ui->reboot_button, TRUE);
    if (!last_error.empty())
    {
        set_status(ui, "Reboot request failed: " + last_error);
        return;
    }
    set_status(ui, "Provisioning completed. Please reboot the system before using Saturn.");
}

static void maybe_prompt_reboot(UiState *ui)
{
    if (ui->reboot_prompt_shown)
    {
        return;
    }

    ui->reboot_prompt_shown = true;
    gtk_widget_set_sensitive(ui->reboot_button, TRUE);

    GtkWidget *dialog = gtk_message_dialog_new(
        GTK_WINDOW(ui->window),
        static_cast<GtkDialogFlags>(GTK_DIALOG_MODAL | GTK_DIALOG_DESTROY_WITH_PARENT),
        GTK_MESSAGE_QUESTION,
        GTK_BUTTONS_NONE,
        "%s",
        "Provisioning completed successfully. A reboot is recommended before using Saturn.");
    gtk_message_dialog_format_secondary_text(
        GTK_MESSAGE_DIALOG(dialog),
        "%s",
        "Would you like to reboot now?");
    gtk_dialog_add_button(GTK_DIALOG(dialog), "Later", GTK_RESPONSE_CANCEL);
    gtk_dialog_add_button(GTK_DIALOG(dialog), "Reboot Now", GTK_RESPONSE_ACCEPT);
    gtk_dialog_set_default_response(GTK_DIALOG(dialog), GTK_RESPONSE_ACCEPT);

    const gint response = gtk_dialog_run(GTK_DIALOG(dialog));
    gtk_widget_destroy(dialog);

    if (response == GTK_RESPONSE_ACCEPT)
    {
        request_reboot(ui);
    }
    else
    {
        set_status(ui, "Provisioning completed. Please reboot the system before using Saturn.");
    }
}

static gboolean on_tick(gpointer user_data)
{
    UiState *ui = static_cast<UiState *>(user_data);
    append_log_delta(ui);

    const gint64 now_us = g_get_monotonic_time();
    const guint64 elapsed = static_cast<guint64>((now_us - ui->start_us) / G_USEC_PER_SEC);
    std::string timer_text = "Elapsed: " + format_duration(elapsed);
    gtk_label_set_text(GTK_LABEL(ui->timer_label), timer_text.c_str());

    std::string status_state;
    std::string status_message;
    const bool has_status = read_status_line(ui->status_file, &status_state, &status_message);
    const bool has_completion = file_exists(ui->completion_file);

    if (!ui->finished)
    {
        if (has_status && status_state == "FAILED")
        {
            ui->finished = true;
            gtk_progress_bar_set_fraction(GTK_PROGRESS_BAR(ui->progress_bar), 1.0);
            set_result(ui, "<span foreground='#ff7f7f' weight='bold' size='x-large'>Provisioning failed</span>");
            set_status(ui, status_message.empty() ? "A provisioning error occurred." : status_message);
        }
        else if (has_status && status_state == "SKIPPED")
        {
            ui->finished = true;
            gtk_progress_bar_set_fraction(GTK_PROGRESS_BAR(ui->progress_bar), 1.0);
            set_result(ui, "<span foreground='#f4d35e' weight='bold' size='x-large'>Provisioning skipped</span>");
            set_status(ui, status_message.empty() ? "System already provisioned. No new run executed." : status_message);
        }
        else if (has_completion || (has_status && status_state == "SUCCESS"))
        {
            ui->finished = true;
            gtk_progress_bar_set_fraction(GTK_PROGRESS_BAR(ui->progress_bar), 1.0);
            set_result(ui, "<span foreground='#8bf58b' weight='bold' size='x-large'>Provisioning successful</span>");
            set_status(ui, status_message.empty() ? "All provisioning steps completed. A reboot is recommended before using Saturn." : status_message + " Reboot is recommended before using Saturn.");
        }
        else
        {
            if (has_status && !status_message.empty())
            {
                set_status(ui, status_message);
                const int stage_index = provisioning_stage_index(status_message);
                if (stage_index >= 0)
                {
                    constexpr int stage_count = 29;
                    const double fraction = static_cast<double>(stage_index + 1) / stage_count;
                    gtk_progress_bar_set_fraction(GTK_PROGRESS_BAR(ui->progress_bar), fraction);
                    const std::string progress_text = "Stage " + std::to_string(stage_index + 1) +
                                                      " of " + std::to_string(stage_count);
                    gtk_progress_bar_set_text(GTK_PROGRESS_BAR(ui->progress_bar), progress_text.c_str());
                }
            }
            else
            {
                set_status(ui, "Provisioning is in progress...");
                gtk_progress_bar_pulse(GTK_PROGRESS_BAR(ui->progress_bar));
            }
        }
    }

    if (ui->finished)
    {
        if (has_completion || (has_status && status_state == "SUCCESS"))
        {
            maybe_prompt_reboot(ui);
        }
        gtk_widget_set_sensitive(ui->close_button, TRUE);
        gtk_progress_bar_set_text(GTK_PROGRESS_BAR(ui->progress_bar), "Done");
    }
    return G_SOURCE_CONTINUE;
}

static void on_toggle_log(GtkToggleButton *button, gpointer user_data)
{
    UiState *ui = static_cast<UiState *>(user_data);
    const gboolean show = gtk_toggle_button_get_active(button);
    gtk_widget_set_visible(ui->log_scroller, show);
    gtk_button_set_label(GTK_BUTTON(ui->toggle_button), show ? "Hide Live Log" : "Show Live Log");
}

int main(int argc, char **argv)
{
    gchar *arg_log = g_strdup("/var/log/saturn-provision.log");
    gchar *arg_completion = g_strdup("/var/lib/saturn-provision/complete");
    gchar *arg_status = g_strdup("/var/lib/saturn-provision/ui-status");
    gint arg_timeout = 2700;
    gboolean arg_show_log = FALSE;

    GOptionEntry entries[] = {
        {"log-file", 0, 0, G_OPTION_ARG_FILENAME, &arg_log, "Provision log file", "PATH"},
        {"completion-file", 0, 0, G_OPTION_ARG_FILENAME, &arg_completion, "Provision completion marker", "PATH"},
        {"status-file", 0, 0, G_OPTION_ARG_FILENAME, &arg_status, "Provision status file", "PATH"},
        {"timeout-seconds", 0, 0, G_OPTION_ARG_INT, &arg_timeout, "Expected provision duration in seconds", "SECONDS"},
        {"show-log", 0, 0, G_OPTION_ARG_NONE, &arg_show_log, "Show live log panel on startup", nullptr},
        {nullptr, 0, 0, G_OPTION_ARG_NONE, nullptr, nullptr, nullptr}};

    GOptionContext *context = g_option_context_new("- Saturn Provisioning UI");
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
        g_free(arg_completion);
        g_free(arg_status);
        return 1;
    }
    g_option_context_free(context);

    UiState ui{};
    ui.log_file = arg_log ? arg_log : "/var/log/saturn-provision.log";
    ui.completion_file = arg_completion ? arg_completion : "/var/lib/saturn-provision/complete";
    ui.status_file = arg_status ? arg_status : "/var/lib/saturn-provision/ui-status";
    ui.timeout_seconds = arg_timeout > 0 ? static_cast<guint>(arg_timeout) : 2700;
    ui.start_us = g_get_monotonic_time();
    ui.log_offset = 0;
    ui.finished = false;
    ui.reboot_prompt_shown = false;

    g_free(arg_log);
    g_free(arg_completion);
    g_free(arg_status);

    ui.window = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_title(GTK_WINDOW(ui.window), "Saturn Provisioning");
    gtk_window_set_default_size(GTK_WINDOW(ui.window), 860, 540);
    gtk_widget_set_name(ui.window, "provision_window");
    gtk_window_set_position(GTK_WINDOW(ui.window), GTK_WIN_POS_CENTER);

    GtkCssProvider *css_provider = gtk_css_provider_new();
    gtk_css_provider_load_from_data(
        css_provider,
        "#provision_window {"
        "  background-image: linear-gradient(135deg, #0f1f3a 0%, #153b59 45%, #1e5b63 100%);"
        "}"
        "#title_label {"
        "  color: #f6fbff;"
        "  font-size: 28px;"
        "  font-weight: 700;"
        "}"
        "#status_label {"
        "  color: #cfe7ff;"
        "  font-size: 15px;"
        "}"
        "#timer_label {"
        "  color: #9dc7f5;"
        "  font-size: 13px;"
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

    GtkWidget *title_label = gtk_label_new("Saturn Provisioning");
    gtk_widget_set_name(title_label, "title_label");
    gtk_widget_set_halign(title_label, GTK_ALIGN_START);
    gtk_box_pack_start(GTK_BOX(root), title_label, FALSE, FALSE, 0);

    ui.result_label = gtk_label_new(nullptr);
    gtk_widget_set_halign(ui.result_label, GTK_ALIGN_START);
    gtk_label_set_markup(GTK_LABEL(ui.result_label),
                         "<span foreground='#eaf3ff' weight='bold' size='x-large'>Preparing provisioning...</span>");
    gtk_box_pack_start(GTK_BOX(root), ui.result_label, FALSE, FALSE, 0);

    ui.status_label = gtk_label_new("Waiting for provisioning status...");
    gtk_widget_set_name(ui.status_label, "status_label");
    gtk_widget_set_halign(ui.status_label, GTK_ALIGN_START);
    gtk_label_set_line_wrap(GTK_LABEL(ui.status_label), TRUE);
    gtk_box_pack_start(GTK_BOX(root), ui.status_label, FALSE, FALSE, 0);

    ui.timer_label = gtk_label_new("Elapsed: 00:00:00");
    gtk_widget_set_name(ui.timer_label, "timer_label");
    gtk_widget_set_halign(ui.timer_label, GTK_ALIGN_START);
    gtk_box_pack_start(GTK_BOX(root), ui.timer_label, FALSE, FALSE, 0);

    ui.progress_bar = gtk_progress_bar_new();
    gtk_progress_bar_set_pulse_step(GTK_PROGRESS_BAR(ui.progress_bar), 0.03);
    gtk_progress_bar_set_show_text(GTK_PROGRESS_BAR(ui.progress_bar), TRUE);
    gtk_box_pack_start(GTK_BOX(root), ui.progress_bar, FALSE, FALSE, 2);

    GtkWidget *button_row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_box_pack_start(GTK_BOX(root), button_row, FALSE, FALSE, 2);

    ui.toggle_button = gtk_toggle_button_new_with_label("Show Live Log");
    gtk_box_pack_start(GTK_BOX(button_row), ui.toggle_button, FALSE, FALSE, 0);

    ui.reboot_button = gtk_button_new_with_label("Reboot Now");
    gtk_widget_set_sensitive(ui.reboot_button, FALSE);
    gtk_box_pack_end(GTK_BOX(button_row), ui.reboot_button, FALSE, FALSE, 0);

    ui.close_button = gtk_button_new_with_label("Close");
    gtk_widget_set_sensitive(ui.close_button, FALSE);
    gtk_box_pack_end(GTK_BOX(button_row), ui.close_button, FALSE, FALSE, 0);

    ui.log_scroller = gtk_scrolled_window_new(nullptr, nullptr);
    gtk_widget_set_vexpand(ui.log_scroller, TRUE);
    gtk_widget_set_hexpand(ui.log_scroller, TRUE);
    gtk_scrolled_window_set_policy(GTK_SCROLLED_WINDOW(ui.log_scroller), GTK_POLICY_AUTOMATIC, GTK_POLICY_AUTOMATIC);
    gtk_box_pack_start(GTK_BOX(root), ui.log_scroller, TRUE, TRUE, 0);

    ui.log_view = gtk_text_view_new();
    gtk_text_view_set_editable(GTK_TEXT_VIEW(ui.log_view), FALSE);
    gtk_text_view_set_cursor_visible(GTK_TEXT_VIEW(ui.log_view), FALSE);
    gtk_text_view_set_wrap_mode(GTK_TEXT_VIEW(ui.log_view), GTK_WRAP_CHAR);
    gtk_container_add(GTK_CONTAINER(ui.log_scroller), ui.log_view);
    ui.log_buffer = gtk_text_view_get_buffer(GTK_TEXT_VIEW(ui.log_view));

    g_signal_connect(ui.window, "destroy", G_CALLBACK(gtk_main_quit), nullptr);
    g_signal_connect(ui.close_button, "clicked", G_CALLBACK(gtk_main_quit), nullptr);
    g_signal_connect_swapped(ui.reboot_button, "clicked", G_CALLBACK(request_reboot), &ui);
    g_signal_connect(ui.toggle_button, "toggled", G_CALLBACK(on_toggle_log), &ui);

    gtk_widget_show_all(ui.window);

    if (!arg_show_log)
    {
        gtk_toggle_button_set_active(GTK_TOGGLE_BUTTON(ui.toggle_button), FALSE);
        gtk_widget_set_visible(ui.log_scroller, FALSE);
    }
    else
    {
        gtk_toggle_button_set_active(GTK_TOGGLE_BUTTON(ui.toggle_button), TRUE);
    }

    on_tick(&ui);
    g_timeout_add_seconds(1, on_tick, &ui);
    gtk_main();
    return 0;
}
