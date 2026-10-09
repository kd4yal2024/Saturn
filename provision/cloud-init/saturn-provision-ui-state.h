#pragma once

#include <string>

// A completion marker may belong to an earlier installer contract. The live
// status must also say SUCCESS before the UI can offer a reboot.
inline bool saturn_provision_run_succeeded(bool has_completion, bool has_status,
                                           const std::string &status_state)
{
    return has_completion && has_status && status_state == "SUCCESS";
}
