#include "collect_ui_backend.h"

// Generated umbrella: LogosModules (behind modules()) built from
// metadata.json#dependencies — typed wrappers plus typed event accessors.
#include "logos_sdk.h"

#include <QCryptographicHash>
#include <QDir>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QSaveFile>
#include <QUrl>

#include <dlfcn.h>

namespace {

/// This plugin's install directory: where basecamp's sandbox lets the view
/// read files from. Found from the loaded library itself rather than from
/// ui-host's arguments, which are basecamp's to change.
QString pluginDir()
{
    Dl_info info{};
    if (dladdr(reinterpret_cast<const void *>(&pluginDir), &info) && info.dli_fname)
        return QFileInfo(QString::fromUtf8(info.dli_fname)).absolutePath();
    return {};
}

/// One `data:image/...;base64,...` string as a file:// URL to the same bytes,
/// written once under `cache` and named by its SHA-256. Anything else, or a
/// write that fails, comes back unchanged — the view then shows its empty
/// state rather than something wrong.
QString localizeImage(const QString &value, const QDir &cache)
{
    static const QString prefix = QStringLiteral("data:image/");
    const qsizetype comma = value.indexOf(QLatin1Char(','));
    if (!value.startsWith(prefix) || comma < 0)
        return value;
    const QString header = value.left(comma);
    if (!header.endsWith(QStringLiteral(";base64")))
        return value;
    const QString mime = header.mid(5, header.size() - 5 - 7);
    QString ext;
    if (mime == QStringLiteral("image/jpeg"))
        ext = QStringLiteral("jpg");
    else if (mime == QStringLiteral("image/png"))
        ext = QStringLiteral("png");
    else if (mime == QStringLiteral("image/webp"))
        ext = QStringLiteral("webp");
    else
        return value;

    const QByteArray bytes = QByteArray::fromBase64(value.mid(comma + 1).toLatin1());
    if (bytes.isEmpty())
        return value;
    const QString name = QString::fromLatin1(
        QCryptographicHash::hash(bytes, QCryptographicHash::Sha256).toHex()) + QLatin1Char('.') + ext;
    const QString path = cache.filePath(name);
    if (!QFileInfo::exists(path)) {
        QSaveFile file(path);
        if (!file.open(QIODevice::WriteOnly) || file.write(bytes) != bytes.size() || !file.commit())
            return value;
    }
    return QUrl::fromLocalFile(path).toString();
}

QJsonValue localize(const QJsonValue &value, const QDir &cache)
{
    if (value.isString())
        return localizeImage(value.toString(), cache);
    if (value.isArray()) {
        QJsonArray out;
        for (const QJsonValue &item : value.toArray())
            out.append(localize(item, cache));
        return out;
    }
    if (value.isObject()) {
        QJsonObject out;
        const QJsonObject in = value.toObject();
        for (auto it = in.begin(); it != in.end(); ++it)
            out.insert(it.key(), localize(it.value(), cache));
        return out;
    }
    return value;
}

/// A collect_core reply with its images moved out of `data:` URLs. Replies
/// without one are passed through untouched, unparsed.
QString localizeReply(const QString &reply)
{
    if (!reply.contains(QStringLiteral("\"data:image/")))
        return reply;
    const QString dir = pluginDir();
    if (dir.isEmpty())
        return reply;
    QDir cache(dir);
    if (!cache.mkpath(QStringLiteral("cache")) || !cache.cd(QStringLiteral("cache")))
        return reply;
    QJsonParseError error{};
    const QJsonDocument doc = QJsonDocument::fromJson(reply.toUtf8(), &error);
    if (error.error != QJsonParseError::NoError)
        return reply;
    const QJsonValue out = localize(doc.isObject() ? QJsonValue(doc.object()) : QJsonValue(doc.array()), cache);
    return QString::fromUtf8(out.isObject() ? QJsonDocument(out.toObject()).toJson(QJsonDocument::Compact)
                                            : QJsonDocument(out.toArray()).toJson(QJsonDocument::Compact));
}

} // namespace

QString CollectUiBackend::call(QString method, QString request)
{
    // collect_core answers with the {ok, data} / {ok, error} envelope, so a
    // failure here has to look the same as a failure there — otherwise the UI
    // needs two ways to read one call.
    return localizeReply(modules().collect_core.call(method, request));
}

void CollectUiBackend::pickImage(QString purpose, QString title)
{
#ifdef Q_OS_MACOS
    // osascript rather than a Qt dialog: it is its own process, so its
    // chooser comes to the front on its own and goes away completely when it
    // closes. The prompt travels as an argument, never inside the script.
    auto *chooser = new QProcess(this);
    QObject::connect(chooser, &QProcess::finished, this,
                     [this, chooser, purpose](int code, QProcess::ExitStatus status) {
                         QString path;
                         if (status == QProcess::NormalExit && code == 0)
                             path = QString::fromUtf8(chooser->readAllStandardOutput()).trimmed();
                         chooser->deleteLater();
                         // Back to basecamp whether a file was chosen or not.
                         QProcess::startDetached(QStringLiteral("/usr/bin/open"),
                                                 {QStringLiteral("-b"), QStringLiteral("co.logos.LogosBasecamp")});
                         Q_EMIT imagePicked(purpose, path);
                     });
    // A form's photo field takes several at once; everything else one. The
    // answer is one POSIX path per line either way.
    const bool many = purpose == QStringLiteral("formPhotos");
    QStringList script{QStringLiteral("on run argv"), QStringLiteral("activate")};
    if (many) {
        script << QStringLiteral("set picked to choose file with prompt (item 1 of argv) "
                                 "of type {\"public.image\"} with multiple selections allowed")
               << QStringLiteral("set out to \"\"")
               << QStringLiteral("repeat with f in picked")
               << QStringLiteral("set out to out & POSIX path of f & linefeed")
               << QStringLiteral("end repeat")
               << QStringLiteral("return out");
    } else {
        script << QStringLiteral("return POSIX path of (choose file with prompt (item 1 of argv) "
                                 "of type {\"public.image\"})");
    }
    script << QStringLiteral("end run");
    QStringList args;
    for (const QString &line : script)
        args << QStringLiteral("-e") << line;
    args << title;
    chooser->start(QStringLiteral("/usr/bin/osascript"), args);
#else
    Q_UNUSED(title);
    Q_EMIT imagePicked(purpose, QString());
#endif
}

void CollectUiBackend::onContextReady()
{
#ifdef Q_OS_MACOS
    setNativePicker(true);
#endif
    const QString reported = modules().collect_core.session_capabilities();
    setCapabilities(reported);
    setCoreReady(!reported.isEmpty());

    // The dashboard is the landing screen, so seed the list eagerly. Each
    // experiment's detail is fetched on demand through call().
    setDocuments(localizeReply(modules().collect_core.experiments_list()));
}
