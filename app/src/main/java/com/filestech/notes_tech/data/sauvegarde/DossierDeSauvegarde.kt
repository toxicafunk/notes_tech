package com.filestech.notes_tech.data.sauvegarde

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import com.filestech.notes_tech.data.prefs.AppSettings
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import timber.log.Timber
import java.io.IOException
import java.time.Clock
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale
import javax.inject.Inject
import javax.inject.Singleton

/**
 * The folder the encrypted backup is written to without asking (D-036): chosen once, with the system's
 * folder picker — a Nextcloud folder, for example, through the Nextcloud app's storage provider.
 *
 * - **The app keeps its zero-network promise.** It writes to a document of the provider; the provider app
 *   uploads it. No permission is added to the manifest: a tree the user picked is granted by the system.
 * - **A new document every time** (`notes-tech-YYYYMMDD-HHMMSS.ntbackup`): never on top of an old backup
 *   (R-12). Pruning old ones is the user's, by hand.
 * - **The permission is the system's, the pointer is ours.** [retenir] takes the persisted permission
 *   before it records the folder; [oublier] releases it and forgets. A folder whose permission is gone —
 *   provider uninstalled, account logged out, folder deleted — is reported by [nouveauDocument] as
 *   [DossierInaccessibleException], never written around.
 */
@Singleton
class DossierDeSauvegarde @Inject constructor(
    @ApplicationContext private val context: Context,
    private val settings: AppSettings,
    private val clock: Clock,
) {

    /** The remembered folder can no longer be written to. */
    class DossierInaccessibleException(cause: Throwable? = null) :
        IOException("dossier de sauvegarde inaccessible", cause)

    /** The remembered folder, or `null`: ask every time. */
    fun dossier(): Uri? = settings.backupFolderNow()?.let(Uri::parse)

    /**
     * Remembers [arbre], the tree URI `ACTION_OPEN_DOCUMENT_TREE` returned. The previous folder's
     * permission, if any, is released: the app holds one folder at most.
     *
     * @throws SecurityException the provider did not grant a persistable permission — the folder is not
     *   remembered.
     */
    fun retenir(arbre: Uri) {
        val precedent = dossier()
        context.contentResolver.takePersistableUriPermission(arbre, LECTURE_ET_ECRITURE)
        settings.setBackupFolder(arbre.toString())
        if (precedent != null && precedent != arbre) relacher(precedent)
    }

    /** Forgets the folder and releases its permission. Never throws: panic mode calls it. */
    fun oublier() {
        val dossier = dossier()
        settings.clearBackupFolder()
        if (dossier != null) relacher(dossier)
    }

    /** The folder's name as its provider shows it, for the Settings row — `null` if it cannot say. */
    suspend fun nom(): String? = withContext(Dispatchers.IO) {
        val arbre = dossier() ?: return@withContext null
        try {
            context.contentResolver.query(
                racine(arbre),
                arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME),
                null,
                null,
                null,
            )?.use { if (it.moveToFirst()) it.getString(0) else null }
        } catch (e: Exception) {
            Timber.w(e, "nom du dossier de sauvegarde illisible")
            null
        }
    }

    /**
     * A new, empty document for one backup, in the remembered folder.
     *
     * @throws DossierInaccessibleException no folder is remembered, its permission is gone, or the
     *   provider refused to create the document.
     */
    suspend fun nouveauDocument(): Uri = withContext(Dispatchers.IO) {
        val arbre = dossier()
        if (arbre == null || !estAccorde(arbre)) throw DossierInaccessibleException()
        val document = try {
            DocumentsContract.createDocument(context.contentResolver, racine(arbre), TYPE, nomDuFichier())
        } catch (e: Exception) {
            // A provider signals a missing folder or a logged-out account in its own way —
            // `FileNotFoundException`, `IllegalArgumentException`, `SecurityException` — all the same here.
            throw DossierInaccessibleException(e)
        }
        document ?: throw DossierInaccessibleException()
    }

    private fun estAccorde(arbre: Uri): Boolean = context.contentResolver.persistedUriPermissions.any {
        it.uri == arbre && it.isReadPermission && it.isWritePermission
    }

    /** The document that stands for the tree's root: the folder itself. */
    private fun racine(arbre: Uri): Uri =
        DocumentsContract.buildDocumentUriUsingTree(arbre, DocumentsContract.getTreeDocumentId(arbre))

    /**
     * In the phone's local time, as the user reads it in the provider's list. The injected clock is UTC
     * (`DatabaseModule.provideClock`): only its instant is taken.
     */
    private fun nomDuFichier(): String {
        val maintenant = LocalDateTime.ofInstant(clock.instant(), ZoneId.systemDefault())
        return "notes-tech-" + HORODATAGE.format(maintenant) + ".ntbackup"
    }

    private fun relacher(arbre: Uri) {
        try {
            context.contentResolver.releasePersistableUriPermission(arbre, LECTURE_ET_ECRITURE)
        } catch (e: SecurityException) {
            // Already gone: what we wanted.
            Timber.w(e, "permission du dossier de sauvegarde deja relachee")
        }
    }

    private companion object {
        const val LECTURE_ET_ECRITURE = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION

        /** A `.ntbackup` has no type of its own; most providers say this one. */
        const val TYPE = "application/octet-stream"

        val HORODATAGE: DateTimeFormatter = DateTimeFormatter.ofPattern("uuuuMMdd-HHmmss", Locale.ROOT)
    }
}
