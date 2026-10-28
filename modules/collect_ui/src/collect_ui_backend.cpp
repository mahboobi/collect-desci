#include "collect_ui_backend.h"

// Generated umbrella: LogosModules (behind modules()) built from
// metadata.json#dependencies — typed wrappers plus typed event accessors.
#include "logos_sdk.h"

QString CollectUiBackend::call(QString method, QString request)
{
    // collect_core answers with the {ok, data} / {ok, error} envelope, so a
    // failure here has to look the same as a failure there — otherwise the UI
    // needs two ways to read one call.
    return modules().collect_core.call(method, request);
}

void CollectUiBackend::onContextReady()
{
    const QString reported = modules().collect_core.session_capabilities();
    setCapabilities(reported);
    setCoreReady(!reported.isEmpty());

    // Pull the experiment straight away. With COLLECT_FIXTURES=1 this is the
    // embedded example; live it will be the manifest fetched from the link.
    // Either way the view reads the same shape, so nothing above here changes
    // when the fetch becomes real.
    setDocuments(modules().collect_core.experiments_open(QStringLiteral("{}")));
}
