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
 * Derives:
 *   - `CollectUiSimpleSource` — generated from collect_ui.rep; implement its
 *     slots and feed its PROPs, which auto-sync to every QML replica.
 *   - `LogosUiPluginContext` — gives `onContextReady()` and `modules()`.
 */
class CollectUiBackend : public CollectUiSimpleSource,
                         public LogosUiPluginContext
{
public:
    /// Forward a SPEC section 6 call to collect_core verbatim.
    QString call(QString method, QString request) override;

protected:
    /// Ask collect_core what it can do, once it is reachable.
    void onContextReady() override;
};
