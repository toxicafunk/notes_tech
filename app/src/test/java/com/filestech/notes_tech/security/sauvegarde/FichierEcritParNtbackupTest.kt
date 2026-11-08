package com.filestech.notes_tech.security.sauvegarde

import com.google.common.truth.Truth.assertThat
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.assertThrows
import java.io.IOException
import java.io.InputStream

/**
 * A `.ntbackup` written by the desktop tool (`tools/ntbackup`, D-036), read by the app's own code: the
 * header, the key derived from a code typed loosely, the AAD, the tag. The tool's test
 * (`tools/ntbackup/tests/shared_fixture.rs`) writes this very file and reads it back; if the two sides
 * ever disagree on the format, one of the two suites goes red.
 *
 * ⚠️ Argon2id at 64 MiB runs here, once per test: the point is the whole chain, not the cipher alone.
 */
class FichierEcritParNtbackupTest {

    private fun ouvrir(saisie: String): ByteArray {
        val ressource = checkNotNull(javaClass.getResourceAsStream("/sauvegarde/ecrit-par-ntbackup.ntbackup"))
        val octets = ressource.use(InputStream::readBytes)
        val entree = FichierDeSauvegarde.borne(octets.inputStream())
        val (enTete, brut) = FichierDeSauvegarde.lireLEnTete(entree)
        val secret = SecretDeSauvegarde.octets(saisie)
        val cle = FichierDeSauvegarde.cle(secret, enTete)
        return FichierDeSauvegarde.ouvrir(entree, cle, enTete, brut).use(InputStream::readBytes)
    }

    @Test
    fun `the app reads what the desktop tool wrote`() {
        val contenu = ouvrir("K7QM-T2XB-9H4N-RC8V-WZ3P-6JDE").toString(Charsets.UTF_8)

        assertThat(contenu).startsWith("""{"format":"notes-tech-backup","version":1,""")
        assertThat(contenu).contains(""""title":"Écrit par ntbackup"""")
    }

    @Test
    fun `the code canonicalises the same on both sides`() {
        // The tool derived its key from this loose form; the app must reach the same canonical code.
        assertThat(ouvrir("k7qm t2xb 9h4n rc8v wz3p 6jde")).isEqualTo(ouvrir("K7QMT2XB9H4NRC8VWZ3P6JDE"))
    }

    @Test
    fun `another code does not open it`() {
        assertThrows<IOException> { ouvrir("K7QM-T2XB-9H4N-RC8V-WZ3P-6JDF") }
    }
}
