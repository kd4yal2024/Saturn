#include <gtk/gtk.h>

#include <array>
#include <cstdio>
#include <cstdlib>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

namespace
{
struct ProfileRow
{
    std::string id;
    std::string label;
    std::string description;
};

struct AppState
{
    GtkWidget *window{};
    GtkWidget *recommendation_label{};
    GtkWidget *boot_config_label{};
    GtkWidget *selector{};
    GtkWidget *description_label{};
    GtkWidget *current_overlays{};
    GtkWidget *preview_view{};
    GtkWidget *status_label{};
    GtkWidget *apply_button{};
    GtkWidget *restore_button{};
    GtkWidget *backup_selector{};
    GtkWidget *backup_detail_label{};
    GtkWidget *reboot_button{};
    GtkTextBuffer *preview_buffer{};
    std::string repo_root;
    std::string helper_path;
    std::vector<ProfileRow> profiles;
    std::vector<std::string> backups;
};

bool run_command_capture(const std::string &command, std::string *output, int *exit_code)
{
    std::array<char, 4096> buffer{};
    std::string result;

    FILE *pipe = popen(command.c_str(), "r");
    if (!pipe)
    {
      if (exit_code)
      {
          *exit_code = -1;
      }
      return false;
    }

    while (fgets(buffer.data(), static_cast<int>(buffer.size()), pipe) != nullptr)
    {
        result += buffer.data();
    }

    const int status = pclose(pipe);
    if (output)
    {
        *output = result;
    }
    if (exit_code)
    {
        *exit_code = status;
    }
    return status == 0;
}

std::string shell_quote(const std::string &value)
{
    std::string out = "'";
    for (char c : value)
    {
        if (c == '\'')
        {
            out += "'\\''";
        }
        else
        {
            out.push_back(c);
        }
    }
    out += "'";
    return out;
}

std::vector<std::string> split_lines(const std::string &text)
{
    std::vector<std::string> lines;
    std::stringstream ss(text);
    std::string line;
    while (std::getline(ss, line))
    {
        lines.push_back(line);
    }
    return lines;
}

std::string extract_value(const std::vector<std::string> &lines, const std::string &key)
{
    const std::string prefix = key + "=";
    for (const auto &line : lines)
    {
        if (line.rfind(prefix, 0) == 0)
        {
            return line.substr(prefix.size());
        }
    }
    return "";
}

std::string extract_block(const std::vector<std::string> &lines, const std::string &key)
{
    const std::string start = key + "<<EOF";
    bool in_block = false;
    std::string out;

    for (const auto &line : lines)
    {
        if (!in_block)
        {
            if (line == start)
            {
                in_block = true;
            }
            continue;
        }

        if (line == "EOF")
        {
            break;
        }
        out += line;
        out += '\n';
    }
    return out;
}

void set_status(AppState *app, const std::string &message)
{
    gtk_label_set_text(GTK_LABEL(app->status_label), message.c_str());
}

std::string basename_copy(const std::string &path)
{
    const auto slash = path.find_last_of('/');
    if (slash == std::string::npos)
    {
        return path;
    }
    return path.substr(slash + 1);
}

std::string selected_profile_id(AppState *app)
{
    const int index = gtk_combo_box_get_active(GTK_COMBO_BOX(app->selector));
    if (index < 0 || static_cast<std::size_t>(index) >= app->profiles.size())
    {
        return "auto";
    }
    return app->profiles[static_cast<std::size_t>(index)].id;
}

void update_preview(AppState *app)
{
    const std::string profile = selected_profile_id(app);
    const std::string command = shell_quote(app->helper_path) + " preview --profile " + shell_quote(profile) + " 2>&1";

    std::string output;
    int exit_code = 0;
    const bool ok = run_command_capture(command, &output, &exit_code);
    const auto lines = split_lines(output);

    const std::string description = extract_value(lines, "description");
    const std::string preview_block = extract_block(lines, "preview_block");
    gtk_label_set_text(GTK_LABEL(app->description_label), description.c_str());
    gtk_text_buffer_set_text(app->preview_buffer, preview_block.c_str(), -1);

    if (ok)
    {
        set_status(app, "Preview ready. Applying will update config.txt and require a reboot.");
    }
    else
    {
        gtk_text_buffer_set_text(app->preview_buffer, output.c_str(), -1);
        set_status(app, "Preview failed. Check the details shown below.");
    }
}

void refresh_detected_state(AppState *app)
{
    const std::string command = shell_quote(app->helper_path) + " detect 2>&1";
    std::string output;
    int exit_code = 0;
    const bool ok = run_command_capture(command, &output, &exit_code);
    const auto lines = split_lines(output);

    const std::string cm = extract_value(lines, "cm");
    const std::string profile = extract_value(lines, "profile");
    const std::string size = extract_value(lines, "size");
    const std::string source = extract_value(lines, "size_source");
    const std::string front_panel_type = extract_value(lines, "front_panel_type");
    const std::string boot_config = extract_value(lines, "boot_config_path");
    const std::string overlays = extract_block(lines, "current_overlay_lines");

    std::string recommendation = "Detected: cm=" + (cm.empty() ? "unknown" : cm) +
                                 " size=" + (size.empty() ? "unknown" : size) +
                                 " profile=" + (profile.empty() ? "unknown" : profile) +
                                 " source=" + (source.empty() ? "unknown" : source) +
                                 " front_panel=" + (front_panel_type.empty() ? "unknown" : front_panel_type);
    gtk_label_set_text(GTK_LABEL(app->recommendation_label), recommendation.c_str());
    gtk_label_set_text(GTK_LABEL(app->boot_config_label), boot_config.empty() ? "Boot config: unknown" : ("Boot config: " + boot_config).c_str());
    gtk_label_set_text(GTK_LABEL(app->current_overlays), overlays.empty() ? "(none)" : overlays.c_str());

    if (ok)
    {
        set_status(app, "Current LCD state refreshed.");
    }
    else
    {
        set_status(app, "Detection failed. The tool can still preview explicit profiles.");
    }
}

void refresh_backups(AppState *app)
{
    std::string output;
    int exit_code = 0;
    app->backups.clear();
    gtk_combo_box_text_remove_all(GTK_COMBO_BOX_TEXT(app->backup_selector));

    if (!run_command_capture(shell_quote(app->helper_path) + " backups 2>/dev/null", &output, &exit_code))
    {
        gtk_combo_box_text_append_text(GTK_COMBO_BOX_TEXT(app->backup_selector), "No backups found");
        gtk_combo_box_set_active(GTK_COMBO_BOX(app->backup_selector), 0);
        gtk_label_set_text(GTK_LABEL(app->backup_detail_label), "Selected backup: none");
        gtk_widget_set_sensitive(app->restore_button, FALSE);
        return;
    }

    for (const auto &line : split_lines(output))
    {
        const std::string path = line;
        if (path.empty())
        {
            continue;
        }
        app->backups.push_back(path);
        gtk_combo_box_text_append_text(GTK_COMBO_BOX_TEXT(app->backup_selector), basename_copy(path).c_str());
    }

    if (app->backups.empty())
    {
        gtk_combo_box_text_append_text(GTK_COMBO_BOX_TEXT(app->backup_selector), "No backups found");
        gtk_combo_box_set_active(GTK_COMBO_BOX(app->backup_selector), 0);
        gtk_label_set_text(GTK_LABEL(app->backup_detail_label), "Selected backup: none");
        gtk_widget_set_sensitive(app->restore_button, FALSE);
    }
    else
    {
        gtk_combo_box_set_active(GTK_COMBO_BOX(app->backup_selector), static_cast<int>(app->backups.size() - 1));
        gtk_label_set_text(GTK_LABEL(app->backup_detail_label), ("Selected backup: " + app->backups.back()).c_str());
        gtk_widget_set_sensitive(app->restore_button, TRUE);
    }
}

void on_refresh_clicked(GtkButton *, gpointer user_data)
{
    auto *app = static_cast<AppState *>(user_data);
    refresh_detected_state(app);
    refresh_backups(app);
    update_preview(app);
}

void on_profile_changed(GtkComboBox *, gpointer user_data)
{
    auto *app = static_cast<AppState *>(user_data);
    update_preview(app);
}

void on_backup_changed(GtkComboBox *, gpointer user_data)
{
    auto *app = static_cast<AppState *>(user_data);
    const int backup_index = gtk_combo_box_get_active(GTK_COMBO_BOX(app->backup_selector));
    if (backup_index < 0 || static_cast<std::size_t>(backup_index) >= app->backups.size())
    {
        gtk_label_set_text(GTK_LABEL(app->backup_detail_label), "Selected backup: none");
        gtk_widget_set_sensitive(app->restore_button, FALSE);
        return;
    }

    gtk_label_set_text(
        GTK_LABEL(app->backup_detail_label),
        ("Selected backup: " + app->backups[static_cast<std::size_t>(backup_index)]).c_str());
    gtk_widget_set_sensitive(app->restore_button, TRUE);
}

void request_reboot(AppState *app)
{
    GError *error = nullptr;
    const gchar *commands[] = {
        "pkexec /usr/bin/systemctl reboot",
        "/usr/bin/systemctl reboot",
    };

    for (const gchar *command : commands)
    {
        if (g_spawn_command_line_async(command, &error))
        {
            set_status(app, "Reboot requested. The system should restart shortly.");
            return;
        }
        if (error)
        {
            g_error_free(error);
            error = nullptr;
        }
    }

    set_status(app, "Failed to request reboot automatically. Please reboot manually.");
}

void on_reboot_clicked(GtkButton *, gpointer user_data)
{
    request_reboot(static_cast<AppState *>(user_data));
}

void on_apply_clicked(GtkButton *, gpointer user_data)
{
    auto *app = static_cast<AppState *>(user_data);
    const std::string profile = selected_profile_id(app);

    GtkWidget *dialog = gtk_message_dialog_new(
        GTK_WINDOW(app->window),
        static_cast<GtkDialogFlags>(GTK_DIALOG_MODAL | GTK_DIALOG_DESTROY_WITH_PARENT),
        GTK_MESSAGE_WARNING,
        GTK_BUTTONS_NONE,
        "%s",
        "Applying an LCD profile will update config.txt.");
    gtk_message_dialog_format_secondary_text(
        GTK_MESSAGE_DIALOG(dialog),
        "%s",
        "A backup will be created first. A reboot is required after applying the profile.");
    gtk_dialog_add_button(GTK_DIALOG(dialog), "Cancel", GTK_RESPONSE_CANCEL);
    gtk_dialog_add_button(GTK_DIALOG(dialog), "Apply Profile", GTK_RESPONSE_ACCEPT);
    const gint response = gtk_dialog_run(GTK_DIALOG(dialog));
    gtk_widget_destroy(dialog);
    if (response != GTK_RESPONSE_ACCEPT)
    {
        set_status(app, "Apply cancelled.");
        return;
    }

    const std::string command = "pkexec " + shell_quote(app->helper_path) + " apply --profile " + shell_quote(profile) + " 2>&1";
    std::string output;
    int exit_code = 0;
    const bool ok = run_command_capture(command, &output, &exit_code);

    if (!ok)
    {
        GtkWidget *error_dialog = gtk_message_dialog_new(
            GTK_WINDOW(app->window),
            static_cast<GtkDialogFlags>(GTK_DIALOG_MODAL | GTK_DIALOG_DESTROY_WITH_PARENT),
            GTK_MESSAGE_ERROR,
            GTK_BUTTONS_CLOSE,
            "%s",
            "Failed to apply the LCD profile.");
        gtk_message_dialog_format_secondary_text(GTK_MESSAGE_DIALOG(error_dialog), "%s", output.c_str());
        gtk_dialog_run(GTK_DIALOG(error_dialog));
        gtk_widget_destroy(error_dialog);
        set_status(app, "Profile apply failed.");
        return;
    }

    gtk_widget_set_sensitive(app->reboot_button, TRUE);
    refresh_detected_state(app);
    update_preview(app);

    GtkWidget *ok_dialog = gtk_message_dialog_new(
        GTK_WINDOW(app->window),
        static_cast<GtkDialogFlags>(GTK_DIALOG_MODAL | GTK_DIALOG_DESTROY_WITH_PARENT),
        GTK_MESSAGE_INFO,
        GTK_BUTTONS_NONE,
        "%s",
        "LCD profile applied successfully.");
    gtk_message_dialog_format_secondary_text(
        GTK_MESSAGE_DIALOG(ok_dialog),
        "%s",
        "config.txt was updated and backed up. Reboot now or later from this window.");
    gtk_dialog_add_button(GTK_DIALOG(ok_dialog), "Close", GTK_RESPONSE_CLOSE);
    gtk_dialog_add_button(GTK_DIALOG(ok_dialog), "Reboot Now", GTK_RESPONSE_ACCEPT);
    const gint reboot_response = gtk_dialog_run(GTK_DIALOG(ok_dialog));
    gtk_widget_destroy(ok_dialog);
    set_status(app, "LCD profile applied. Reboot is required before using the new panel settings.");
    if (reboot_response == GTK_RESPONSE_ACCEPT)
    {
        request_reboot(app);
    }
}

void on_restore_clicked(GtkButton *, gpointer user_data)
{
    auto *app = static_cast<AppState *>(user_data);
    const int backup_index = gtk_combo_box_get_active(GTK_COMBO_BOX(app->backup_selector));
    if (backup_index < 0 || static_cast<std::size_t>(backup_index) >= app->backups.size())
    {
        set_status(app, "No LCD backup is available to restore.");
        return;
    }
    const std::string selected_backup = app->backups[static_cast<std::size_t>(backup_index)];

    GtkWidget *dialog = gtk_message_dialog_new(
        GTK_WINDOW(app->window),
        static_cast<GtkDialogFlags>(GTK_DIALOG_MODAL | GTK_DIALOG_DESTROY_WITH_PARENT),
        GTK_MESSAGE_WARNING,
        GTK_BUTTONS_NONE,
        "%s",
        "Restore the selected LCD config backup?");
    gtk_message_dialog_format_secondary_text(
        GTK_MESSAGE_DIALOG(dialog),
        "%s",
        ("This will copy the selected backup over config.txt:\n" + selected_backup + "\n\nA reboot is required afterward.").c_str());
    gtk_dialog_add_button(GTK_DIALOG(dialog), "Cancel", GTK_RESPONSE_CANCEL);
    gtk_dialog_add_button(GTK_DIALOG(dialog), "Restore Backup", GTK_RESPONSE_ACCEPT);
    const gint response = gtk_dialog_run(GTK_DIALOG(dialog));
    gtk_widget_destroy(dialog);
    if (response != GTK_RESPONSE_ACCEPT)
    {
        set_status(app, "Restore cancelled.");
        return;
    }

    const std::string command = "pkexec " + shell_quote(app->helper_path) + " restore --backup " + shell_quote(selected_backup) + " 2>&1";
    std::string output;
    int exit_code = 0;
    const bool ok = run_command_capture(command, &output, &exit_code);
    if (!ok)
    {
        GtkWidget *error_dialog = gtk_message_dialog_new(
            GTK_WINDOW(app->window),
            static_cast<GtkDialogFlags>(GTK_DIALOG_MODAL | GTK_DIALOG_DESTROY_WITH_PARENT),
            GTK_MESSAGE_ERROR,
            GTK_BUTTONS_CLOSE,
            "%s",
            "Failed to restore the LCD config backup.");
        gtk_message_dialog_format_secondary_text(GTK_MESSAGE_DIALOG(error_dialog), "%s", output.c_str());
        gtk_dialog_run(GTK_DIALOG(error_dialog));
        gtk_widget_destroy(error_dialog);
        set_status(app, "Restore failed.");
        return;
    }

    gtk_widget_set_sensitive(app->reboot_button, TRUE);
    refresh_detected_state(app);
    refresh_backups(app);
    update_preview(app);
    set_status(app, "Backup restored. Reboot is required before using the restored panel settings.");
}

std::vector<ProfileRow> load_profiles(const std::string &helper_path)
{
    std::vector<ProfileRow> profiles;
    std::string output;
    int exit_code = 0;
    if (!run_command_capture(shell_quote(helper_path) + " profiles 2>&1", &output, &exit_code))
    {
        return profiles;
    }

    for (const auto &line : split_lines(output))
    {
        std::stringstream ss(line);
        ProfileRow row;
        if (!std::getline(ss, row.id, '|'))
        {
            continue;
        }
        if (!std::getline(ss, row.label, '|'))
        {
            continue;
        }
        if (!std::getline(ss, row.description))
        {
            row.description.clear();
        }
        profiles.push_back(row);
    }
    return profiles;
}
} // namespace

int main(int argc, char **argv)
{
    gtk_init(&argc, &argv);

    AppState app;
    app.repo_root = std::getenv("SATURN_ROOT") ? std::getenv("SATURN_ROOT") : (std::string(std::getenv("HOME")) + "/github/Saturn");
    app.helper_path = app.repo_root + "/scripts/saturn-lcd-helper.sh";
    app.profiles = load_profiles(app.helper_path);
    if (app.profiles.empty())
    {
        g_printerr("Could not load LCD profiles from %s\n", app.helper_path.c_str());
        return 1;
    }

    app.window = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_title(GTK_WINDOW(app.window), "Saturn LCD Setup");
    gtk_window_set_icon_name(GTK_WINDOW(app.window), "saturn-lcd-setup");
    gtk_window_set_default_size(GTK_WINDOW(app.window), 900, 700);
    gtk_window_set_position(GTK_WINDOW(app.window), GTK_WIN_POS_CENTER);

    GtkCssProvider *css_provider = gtk_css_provider_new();
    gtk_css_provider_load_from_data(
        css_provider,
        "window { background-image: linear-gradient(135deg, #f3efe3 0%, #dce7ea 100%); }"
        "label#title { font-size: 28px; font-weight: 700; color: #162027; }"
        "label#subtle { color: #41545f; }"
        "textview { font-family: Monospace; font-size: 10.5pt; }",
        -1,
        nullptr);
    gtk_style_context_add_provider_for_screen(gdk_screen_get_default(), GTK_STYLE_PROVIDER(css_provider), GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
    g_object_unref(css_provider);

    GtkWidget *root = gtk_box_new(GTK_ORIENTATION_VERTICAL, 12);
    gtk_container_set_border_width(GTK_CONTAINER(root), 18);
    gtk_container_add(GTK_CONTAINER(app.window), root);

    GtkWidget *title = gtk_label_new("Saturn LCD Setup");
    gtk_widget_set_name(title, "title");
    gtk_widget_set_halign(title, GTK_ALIGN_START);
    gtk_box_pack_start(GTK_BOX(root), title, FALSE, FALSE, 0);

    GtkWidget *subtitle = gtk_label_new("Inspect the current LCD wiring/config, preview known profiles, apply one safely, then reboot.");
    gtk_widget_set_name(subtitle, "subtle");
    gtk_widget_set_halign(subtitle, GTK_ALIGN_START);
    gtk_label_set_line_wrap(GTK_LABEL(subtitle), TRUE);
    gtk_box_pack_start(GTK_BOX(root), subtitle, FALSE, FALSE, 0);

    GtkWidget *grid = gtk_grid_new();
    gtk_grid_set_row_spacing(GTK_GRID(grid), 10);
    gtk_grid_set_column_spacing(GTK_GRID(grid), 12);
    gtk_box_pack_start(GTK_BOX(root), grid, FALSE, FALSE, 0);

    GtkWidget *detected_label = gtk_label_new("Detected State:");
    gtk_widget_set_halign(detected_label, GTK_ALIGN_START);
    gtk_grid_attach(GTK_GRID(grid), detected_label, 0, 0, 1, 1);

    app.recommendation_label = gtk_label_new("");
    gtk_widget_set_halign(app.recommendation_label, GTK_ALIGN_START);
    gtk_label_set_line_wrap(GTK_LABEL(app.recommendation_label), TRUE);
    gtk_grid_attach(GTK_GRID(grid), app.recommendation_label, 1, 0, 1, 1);

    GtkWidget *boot_label = gtk_label_new("Config File:");
    gtk_widget_set_halign(boot_label, GTK_ALIGN_START);
    gtk_grid_attach(GTK_GRID(grid), boot_label, 0, 1, 1, 1);

    app.boot_config_label = gtk_label_new("");
    gtk_widget_set_halign(app.boot_config_label, GTK_ALIGN_START);
    gtk_grid_attach(GTK_GRID(grid), app.boot_config_label, 1, 1, 1, 1);

    GtkWidget *profile_label = gtk_label_new("Target Profile:");
    gtk_widget_set_halign(profile_label, GTK_ALIGN_START);
    gtk_grid_attach(GTK_GRID(grid), profile_label, 0, 2, 1, 1);

    app.selector = gtk_combo_box_text_new();
    for (const auto &profile : app.profiles)
    {
        gtk_combo_box_text_append_text(GTK_COMBO_BOX_TEXT(app.selector), profile.label.c_str());
    }
    gtk_combo_box_set_active(GTK_COMBO_BOX(app.selector), 0);
    gtk_grid_attach(GTK_GRID(grid), app.selector, 1, 2, 1, 1);

    app.description_label = gtk_label_new("");
    gtk_widget_set_halign(app.description_label, GTK_ALIGN_START);
    gtk_label_set_line_wrap(GTK_LABEL(app.description_label), TRUE);
    gtk_box_pack_start(GTK_BOX(root), app.description_label, FALSE, FALSE, 0);

    GtkWidget *panes = gtk_paned_new(GTK_ORIENTATION_VERTICAL);
    gtk_widget_set_vexpand(panes, TRUE);
    gtk_box_pack_start(GTK_BOX(root), panes, TRUE, TRUE, 0);

    GtkWidget *current_frame = gtk_frame_new("Current Waveshare Overlay Lines");
    GtkWidget *current_box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 6);
    gtk_container_add(GTK_CONTAINER(current_frame), current_box);
    app.current_overlays = gtk_label_new("");
    gtk_widget_set_halign(app.current_overlays, GTK_ALIGN_START);
    gtk_label_set_selectable(GTK_LABEL(app.current_overlays), TRUE);
    gtk_label_set_line_wrap(GTK_LABEL(app.current_overlays), TRUE);
    gtk_box_pack_start(GTK_BOX(current_box), app.current_overlays, FALSE, FALSE, 8);
    gtk_paned_pack1(GTK_PANED(panes), current_frame, FALSE, TRUE);

    GtkWidget *preview_frame = gtk_frame_new("Managed LCD Block Preview");
    GtkWidget *preview_scroll = gtk_scrolled_window_new(nullptr, nullptr);
    gtk_container_add(GTK_CONTAINER(preview_frame), preview_scroll);
    app.preview_view = gtk_text_view_new();
    gtk_text_view_set_editable(GTK_TEXT_VIEW(app.preview_view), FALSE);
    gtk_text_view_set_cursor_visible(GTK_TEXT_VIEW(app.preview_view), FALSE);
    gtk_container_add(GTK_CONTAINER(preview_scroll), app.preview_view);
    app.preview_buffer = gtk_text_view_get_buffer(GTK_TEXT_VIEW(app.preview_view));
    gtk_paned_pack2(GTK_PANED(panes), preview_frame, TRUE, TRUE);

    GtkWidget *button_row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 8);
    gtk_box_pack_start(GTK_BOX(root), button_row, FALSE, FALSE, 0);

    GtkWidget *refresh_button = gtk_button_new_with_label("Refresh");
    gtk_box_pack_start(GTK_BOX(button_row), refresh_button, FALSE, FALSE, 0);

    app.apply_button = gtk_button_new_with_label("Apply Profile");
    gtk_box_pack_start(GTK_BOX(button_row), app.apply_button, FALSE, FALSE, 0);

    app.backup_selector = gtk_combo_box_text_new();
    gtk_widget_set_size_request(app.backup_selector, 260, -1);
    gtk_box_pack_start(GTK_BOX(button_row), app.backup_selector, FALSE, FALSE, 0);

    app.restore_button = gtk_button_new_with_label("Restore Selected Backup");
    gtk_box_pack_start(GTK_BOX(button_row), app.restore_button, FALSE, FALSE, 0);

    app.reboot_button = gtk_button_new_with_label("Reboot Now");
    gtk_widget_set_sensitive(app.reboot_button, FALSE);
    gtk_box_pack_start(GTK_BOX(button_row), app.reboot_button, FALSE, FALSE, 0);

    GtkWidget *close_button = gtk_button_new_with_label("Close");
    gtk_box_pack_end(GTK_BOX(button_row), close_button, FALSE, FALSE, 0);

    app.status_label = gtk_label_new("Ready.");
    gtk_widget_set_halign(app.status_label, GTK_ALIGN_START);
    gtk_label_set_line_wrap(GTK_LABEL(app.status_label), TRUE);
    gtk_box_pack_start(GTK_BOX(root), app.status_label, FALSE, FALSE, 0);

    app.backup_detail_label = gtk_label_new("Selected backup: none");
    gtk_widget_set_halign(app.backup_detail_label, GTK_ALIGN_START);
    gtk_label_set_line_wrap(GTK_LABEL(app.backup_detail_label), TRUE);
    gtk_box_pack_start(GTK_BOX(root), app.backup_detail_label, FALSE, FALSE, 0);

    g_signal_connect(app.window, "destroy", G_CALLBACK(gtk_main_quit), nullptr);
    g_signal_connect(close_button, "clicked", G_CALLBACK(gtk_main_quit), nullptr);
    g_signal_connect(refresh_button, "clicked", G_CALLBACK(on_refresh_clicked), &app);
    g_signal_connect(app.selector, "changed", G_CALLBACK(on_profile_changed), &app);
    g_signal_connect(app.backup_selector, "changed", G_CALLBACK(on_backup_changed), &app);
    g_signal_connect(app.apply_button, "clicked", G_CALLBACK(on_apply_clicked), &app);
    g_signal_connect(app.restore_button, "clicked", G_CALLBACK(on_restore_clicked), &app);
    g_signal_connect(app.reboot_button, "clicked", G_CALLBACK(on_reboot_clicked), &app);

    refresh_detected_state(&app);
    refresh_backups(&app);
    update_preview(&app);

    gtk_widget_show_all(app.window);
    gtk_main();
    return 0;
}
