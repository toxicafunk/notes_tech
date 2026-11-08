package com.filestech.notes_tech.ui.settings

import android.net.Uri
import com.filestech.notes_tech.R
import com.filestech.notes_tech.data.prefs.AppSettings
import com.filestech.notes_tech.data.sauvegarde.DossierDeSauvegarde
import com.filestech.notes_tech.data.sauvegarde.RestaurationDeSauvegarde
import com.filestech.notes_tech.data.sauvegarde.SauvegardeChiffree
import com.google.common.truth.Truth.assertThat
import io.mockk.coEvery
import io.mockk.coVerify
import io.mockk.every
import io.mockk.just
import io.mockk.mockk
import io.mockk.runs
import io.mockk.verify
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.BeforeEach
import org.junit.jupiter.api.DisplayName
import org.junit.jupiter.api.Test
import java.io.File

/**
 * The remembered backup folder (D-036) in the creation flow: written there without a picker; a folder
 * gone said, forgotten, and no encrypted file left waiting; no folder, the picker as before.
 */
class SauvegardeViewModelTest {

    private val sauvegarde = mockk<SauvegardeChiffree>()
    private val restauration = mockk<RestaurationDeSauvegarde>()
    private val dossier = mockk<DossierDeSauvegarde>()
    private val settings = mockk<AppSettings>()

    private val apercu = SauvegardeChiffree.Apercu(
        dossiers = 2,
        notes = 3,
        coffresACodeFermes = emptyList(),
        coffresACodeOuverts = emptyList(),
        idsDesCoffresACodeOuverts = emptySet(),
    )
    private val creation = SauvegardeChiffree.Creation(dossiers = 2, notes = 3, omises = 0)
    private val preparee = SauvegardeChiffree.Preparee(File("attente.ntbackup"), creation)
    private val document = mockk<Uri>()

    private lateinit var viewModel: SauvegardeViewModel

    @BeforeEach
    fun setUp() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
        every { settings.backupFolder } returns flowOf(null)
        coEvery { sauvegarde.apercu() } returns apercu
        coEvery { sauvegarde.preparer(any(), any()) } returns preparee
        every { sauvegarde.abandonner(any()) } just runs
        every { dossier.oublier() } just runs
        viewModel = SauvegardeViewModel(sauvegarde, restauration, dossier, settings)
    }

    @AfterEach
    fun tearDown() = Dispatchers.resetMain()

    private fun creer() {
        viewModel.preparerLaCreation()
        assertThat(viewModel.etape.value).isInstanceOf(SauvegardeViewModel.Etape.AvantLaCreation::class.java)
        viewModel.secretChoisi("K7QM-T2XB-9H4N-RC8V-WZ3P-6JDE")
    }

    @Test
    @DisplayName("a remembered folder: written there, no picker")
    fun dansLeDossierRetenu() {
        every { dossier.dossier() } returns mockk()
        coEvery { dossier.nouveauDocument() } returns document
        coEvery { sauvegarde.deposer(preparee, document) } returns creation

        creer()

        assertThat(viewModel.etape.value).isEqualTo(SauvegardeViewModel.Etape.Creee(creation))
        coVerify(exactly = 1) { sauvegarde.deposer(preparee, document) }
    }

    @Test
    @DisplayName("the folder gone: said, forgotten, and nothing left waiting")
    fun dossierDisparu() {
        every { dossier.dossier() } returns mockk()
        coEvery { dossier.nouveauDocument() } throws DossierDeSauvegarde.DossierInaccessibleException()

        creer()

        assertThat(viewModel.etape.value).isEqualTo(
            SauvegardeViewModel.Etape.Echec(R.string.backup_create, R.string.backup_folder_error_gone),
        )
        verify(exactly = 1) { dossier.oublier() }
        verify(exactly = 1) { sauvegarde.abandonner(preparee) }
        coVerify(exactly = 0) { sauvegarde.deposer(any(), any()) }
    }

    @Test
    @DisplayName("no folder: the picker, as before")
    fun sansDossier() {
        every { dossier.dossier() } returns null

        creer()

        assertThat(viewModel.etape.value).isEqualTo(SauvegardeViewModel.Etape.PreteADeposer)
        verify(exactly = 0) { sauvegarde.abandonner(any()) }
    }

    @Test
    @DisplayName("a provider that grants no lasting permission: said, nothing remembered")
    fun permissionRefusee() {
        every { dossier.retenir(document) } throws SecurityException("no persistable grant")

        viewModel.dossierChoisi(document)

        assertThat(viewModel.etape.value).isEqualTo(
            SauvegardeViewModel.Etape.Echec(R.string.backup_folder, R.string.backup_folder_error_permission),
        )
    }

    @Test
    @DisplayName("the folder picker dismissed: nothing changes")
    fun selecteurAnnule() {
        viewModel.dossierChoisi(null)

        verify(exactly = 0) { dossier.retenir(any()) }
        assertThat(viewModel.etape.value).isEqualTo(SauvegardeViewModel.Etape.Repos)
    }
}
