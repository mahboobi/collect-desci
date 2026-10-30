#pragma once

#include "rep_collect_ui_source.h"
#include "logos_ui_plugin_context.h"

/**
 * @brief The Collect UI backend.
 *
 * Deliberately thin. Every rule Collect has lives in collect_core, which is
 * reached through the typed caller `LogosUiPluginContext` exposes for each
 * entry in metadata.json#dependencies. This class exists only because a
 * `ui-qml-backend` module reaches its view through a QtRO replica rather than
 * through callModule (SPEC Q12 asks whether it should exist at all).
 *
 * Two jobs the sandboxed view cannot do for itself also live here, because
 * this code runs outside the QML sandbox:
 *   - images: basecamp's sandbox refuses `data:` URLs, so every image in a
 *     reply is written into this module's own directory (an allowed root)
 *     and handed to the view as a `file://` URL;
 *   - picking a file: a native dialog opened from the view is parented to an
 *     offscreen window and strands the person outside basecamp, so on macOS
 *     the chooser runs in its own short-lived process and basecamp is brought
 *     back to the front when it closes.
 *
 * Derives:
 *   - `CollectUiSimpleSource` — generated from collect_ui.rep; implement its
 *     slots and feed its PROPs, which auto-sync to every QML replica.
 *   - `LogosUiPluginContext` — gives `onContextReady()` and `modules()`.
 */
class CollectUiBackend : public CollectUiSimpleSource,
                         public LogosUiPluginContext
{
public:
    /// Forward a SPEC section 6 call to collect_core, with its images made
    /// loadable by the sandboxed view.
    QString call(QString method, QString request) override;

    /// Open the system image chooser; answers through `imagePicked`.
    void pickImage(QString purpose, QString title) override;

protected:
    /// Ask collect_core what it can do, once it is reachable.
    void onContextReady() override;
};
