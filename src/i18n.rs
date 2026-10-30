use serde::{Deserialize, Serialize};

use crate::model::LabelerBackend;

/// Supported interface languages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    English,
    French,
}

impl Default for Language {
    fn default() -> Self {
        // French by default for the French version of the app.
        Language::French
    }
}

#[allow(dead_code)]
impl Language {
    pub const ALL: &'static [Language] = &[Language::French, Language::English];

    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::French => "fr",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::French => "Français",
        }
    }

    pub fn short_name(self) -> &'static str {
        match self {
            Self::English => "EN",
            Self::French => "FR",
        }
    }

    // App header & brand
    pub fn app_title(self) -> &'static str {
        match self {
            Self::English => "PoSTier - Mail Explorer and Local RAG",
            Self::French => "PoSTier - Explorateur de courriels et RAG local",
        }
    }

    pub fn brand_subtitle(self) -> &'static str {
        match self {
            Self::English => "PST Mail Explorer & Triage",
            Self::French => "Explorateur et tri de courriels PST",
        }
    }

    pub fn language_label(self) -> &'static str {
        match self {
            Self::English => "Language",
            Self::French => "Langue",
        }
    }

    pub fn theme_label(self) -> &'static str {
        match self {
            Self::English => "Theme",
            Self::French => "Thème",
        }
    }

    pub fn theme_dark(self) -> &'static str {
        match self {
            Self::English => "Dark",
            Self::French => "Sombre",
        }
    }

    pub fn theme_light(self) -> &'static str {
        match self {
            Self::English => "Light",
            Self::French => "Clair",
        }
    }

    // Sidebar sections
    pub fn section_mailboxes(self) -> &'static str {
        match self {
            Self::English => "MAILBOXES AND FOLDERS",
            Self::French => "BOÎTES ET DOSSIERS",
        }
    }

    pub fn section_labels(self) -> &'static str {
        match self {
            Self::English => "MAIL LABELS",
            Self::French => "ÉTIQUETTES DE COURRIEL",
        }
    }

    pub fn section_settings(self) -> &'static str {
        match self {
            Self::English => "Settings",
            Self::French => "Paramètres",
        }
    }

    pub fn section_laya(self) -> &'static str {
        match self {
            Self::English => "LAYA DECISION MODEL",
            Self::French => "MODÈLE DE DÉCISION LAYA",
        }
    }

    pub fn section_ai(self) -> &'static str {
        match self {
            Self::English => "LLM JUDGE AND RAG",
            Self::French => "JUGE LLM ET RAG",
        }
    }

    pub fn section_storage(self) -> &'static str {
        match self {
            Self::English => "STORAGE",
            Self::French => "STOCKAGE",
        }
    }

    // Common buttons
    pub fn btn_hide(self) -> &'static str {
        match self {
            Self::English => "Hide",
            Self::French => "Masquer",
        }
    }

    pub fn btn_open(self) -> &'static str {
        match self {
            Self::English => "Open",
            Self::French => "Ouvrir",
        }
    }

    pub fn btn_new(self) -> &'static str {
        match self {
            Self::English => "New",
            Self::French => "Nouveau",
        }
    }

    pub fn btn_cancel(self) -> &'static str {
        match self {
            Self::English => "Cancel",
            Self::French => "Annuler",
        }
    }

    pub fn btn_remove(self) -> &'static str {
        match self {
            Self::English => "Remove",
            Self::French => "Supprimer",
        }
    }

    pub fn btn_clear(self) -> &'static str {
        match self {
            Self::English => "Clear",
            Self::French => "Effacer",
        }
    }

    pub fn btn_delete(self) -> &'static str {
        match self {
            Self::English => "Delete",
            Self::French => "Supprimer",
        }
    }

    pub fn btn_show(self) -> &'static str {
        match self {
            Self::English => "Show",
            Self::French => "Afficher",
        }
    }

    // Sidebar - Mailboxes
    pub fn all_messages(self) -> &'static str {
        match self {
            Self::English => "All Messages",
            Self::French => "Tous les messages",
        }
    }

    pub fn import_prefix(self, name: &str) -> String {
        match self {
            Self::English => format!("Import: {name}"),
            Self::French => format!("Importer : {name}"),
        }
    }

    pub fn btn_choose_pst(self) -> &'static str {
        match self {
            Self::English => "Choose PST",
            Self::French => "Choisir PST",
        }
    }

    pub fn btn_index_now(self) -> &'static str {
        match self {
            Self::English => "Index now",
            Self::French => "Indexer",
        }
    }

    pub fn no_archive_selected(self) -> &'static str {
        match self {
            Self::English => "No archive selected",
            Self::French => "Aucune archive sélectionnée",
        }
    }

    pub fn no_pst_indexed(self) -> &'static str {
        match self {
            Self::English => "No PST files indexed yet",
            Self::French => "Aucun fichier PST encore indexé",
        }
    }

    pub fn btn_remove_index(self) -> &'static str {
        match self {
            Self::English => "Remove index",
            Self::French => "Supprimer l'index",
        }
    }

    pub fn confirm_remove_pst(self) -> &'static str {
        match self {
            Self::English => "Remove this PST's messages and embeddings?",
            Self::French => "Supprimer les messages et embeddings de ce PST ?",
        }
    }

    // Sidebar - Labels
    pub fn btn_all(self) -> &'static str {
        match self {
            Self::English => "All",
            Self::French => "Tout",
        }
    }

    pub fn unlabeled_count(self, count: u64) -> String {
        match self {
            Self::English => format!("Unlabeled ({count})"),
            Self::French => format!("Sans étiquette ({count})"),
        }
    }

    pub fn add_or_update_label(self) -> &'static str {
        match self {
            Self::English => "Add or update label",
            Self::French => "Ajouter ou modifier une étiquette",
        }
    }

    pub fn label_name_placeholder(self) -> &'static str {
        match self {
            Self::English => "Label name (e.g. Billing)",
            Self::French => "Nom de l'étiquette (ex. Facturation)",
        }
    }

    pub fn label_desc_placeholder(self) -> &'static str {
        match self {
            Self::English => "Criteria description for Laya or LLM Judge",
            Self::French => "Description des critères pour Laya ou Juge LLM",
        }
    }

    pub fn btn_save_label(self) -> &'static str {
        match self {
            Self::English => "Save Label",
            Self::French => "Enregistrer l'étiquette",
        }
    }

    // Sidebar - Laya
    pub fn laya_ready(self, path: &str) -> String {
        match self {
            Self::English => format!("External GGUF ready: {path}"),
            Self::French => format!("GGUF externe prêt : {path}"),
        }
    }

    pub fn laya_builtin_active(self) -> &'static str {
        match self {
            Self::English => "Active: Built-in packaged Laya-GGUF decision model",
            Self::French => "Actif : modèle de décision Laya-GGUF intégré",
        }
    }

    pub fn laya_path_placeholder(self) -> &'static str {
        match self {
            Self::English => "Optional path to Laya-Q8_0.gguf",
            Self::French => "Chemin optionnel vers Laya-Q8_0.gguf",
        }
    }

    pub fn btn_choose_gguf(self) -> &'static str {
        match self {
            Self::English => "Choose GGUF...",
            Self::French => "Choisir GGUF...",
        }
    }

    pub fn btn_download_q8(self) -> &'static str {
        match self {
            Self::English => "Download Q8_0",
            Self::French => "Télécharger Q8_0",
        }
    }

    pub fn laya_explanation(self) -> &'static str {
        match self {
            Self::English => {
                "Packaged Laya-GGUF runs in-process offline. Downloading Laya-Q8_0.gguf switches to the full 421M checkpoint."
            }
            Self::French => {
                "Laya-GGUF intégré s'exécute hors ligne dans le processus. Télécharger Laya-Q8_0.gguf active le point de contrôle complet de 421M."
            }
        }
    }

    // Sidebar - LLM & Storage
    pub fn endpoint_label(self) -> &'static str {
        match self {
            Self::English => "Endpoint",
            Self::French => "Point de terminaison",
        }
    }

    pub fn model_label(self) -> &'static str {
        match self {
            Self::English => "Model",
            Self::French => "Modèle",
        }
    }

    pub fn model_placeholder(self) -> &'static str {
        match self {
            Self::English => "Model name",
            Self::French => "Nom du modèle",
        }
    }

    pub fn api_key_label(self) -> &'static str {
        match self {
            Self::English => "API key",
            Self::French => "Clé API",
        }
    }

    pub fn token_placeholder(self) -> &'static str {
        match self {
            Self::English => "Token",
            Self::French => "Token",
        }
    }

    pub fn remote_embeddings_checkbox(self) -> &'static str {
        match self {
            Self::English => "Remote embeddings on index",
            Self::French => "Embeddings distants lors de l'indexation",
        }
    }

    pub fn embedding_model_label(self) -> &'static str {
        match self {
            Self::English => "Embedding model",
            Self::French => "Modèle d'embedding",
        }
    }

    pub fn embedding_model_placeholder(self) -> &'static str {
        match self {
            Self::English => "Embedding model name",
            Self::French => "Nom du modèle d'embedding",
        }
    }

    pub fn context_emails_label(self) -> &'static str {
        match self {
            Self::English => "Emails retrieved per question",
            Self::French => "Courriels récupérés par question",
        }
    }

    pub fn context_emails_hint(self) -> &'static str {
        match self {
            Self::English => {
                "Top search results sent to the LLM (1–100), plus related thread messages"
            }
            Self::French => {
                "Meilleurs résultats envoyés au LLM (1–100), plus les messages du même fil"
            }
        }
    }

    pub fn https_required_notice(self) -> &'static str {
        match self {
            Self::English => "HTTPS required except for 127.0.0.1",
            Self::French => "HTTPS requis sauf pour 127.0.0.1",
        }
    }

    pub fn btn_save_ai_settings(self) -> &'static str {
        match self {
            Self::English => "Save AI settings",
            Self::French => "Enregistrer les paramètres IA",
        }
    }

    pub fn storage_notice(self) -> &'static str {
        match self {
            Self::English => "Contains SQLite index, labels, models, and credentials",
            Self::French => {
                "Contient l'index SQLite, les étiquettes, les modèles et les identifiants"
            }
        }
    }

    pub fn btn_choose_directory(self) -> &'static str {
        match self {
            Self::English => "Choose directory",
            Self::French => "Choisir un dossier",
        }
    }

    // Explorer - Status bar
    pub fn status_ready(self) -> &'static str {
        match self {
            Self::English => "Ready - PoSTier mail explorer with Laya and LLM Judge labeling",
            Self::French => {
                "Prêt - Explorateur de courriels PoSTier avec étiquetage Laya et Juge LLM"
            }
        }
    }

    pub fn status_showing(
        self,
        shown: usize,
        total: u64,
        selected: usize,
        backend: &str,
    ) -> String {
        match self {
            Self::English => format!(
                "Showing {shown} of {total} indexed, {selected} selected, Labeler: {backend}"
            ),
            Self::French => format!(
                "Affichage de {shown} sur {total} indexés, {selected} sélectionné(s), Étiqueteur : {backend}"
            ),
        }
    }

    pub fn btn_stop_task(self) -> &'static str {
        match self {
            Self::English => "Stop Task",
            Self::French => "Arrêter la tâche",
        }
    }

    // Explorer - Toolbar
    pub fn all_mailboxes(self) -> &'static str {
        match self {
            Self::English => "All Mailboxes",
            Self::French => "Toutes les boîtes",
        }
    }

    pub fn in_view_checked(self, in_view: usize, checked: usize) -> String {
        match self {
            Self::English => format!("{in_view} in view, {checked} checked"),
            Self::French => format!("{in_view} visibles, {checked} cochés"),
        }
    }

    pub fn all_labels(self) -> &'static str {
        match self {
            Self::English => "All labels",
            Self::French => "Toutes les étiquettes",
        }
    }

    pub fn unlabeled(self) -> &'static str {
        match self {
            Self::English => "Unlabeled",
            Self::French => "Sans étiquette",
        }
    }

    pub fn filter_placeholder(self) -> &'static str {
        match self {
            Self::English => "Quick filter (subject, sender, body)... [/]",
            Self::French => "Filtre rapide (sujet, expéditeur, corps)... [/]",
        }
    }

    pub fn btn_hide_chat(self) -> &'static str {
        match self {
            Self::English => "Hide Chat",
            Self::French => "Masquer le chat",
        }
    }

    pub fn btn_open_chat(self) -> &'static str {
        match self {
            Self::English => "Open Chat",
            Self::French => "Ouvrir le chat",
        }
    }

    pub fn btn_show_sidebar(self) -> &'static str {
        match self {
            Self::English => "Show Sidebar",
            Self::French => "Afficher la barre latérale",
        }
    }

    pub fn decision_engine_label(self) -> &'static str {
        match self {
            Self::English => "Decision engine:",
            Self::French => "Moteur de décision :",
        }
    }

    pub fn label_focused_mail(self) -> &'static str {
        match self {
            Self::English => "Label Focused Mail",
            Self::French => "Étiqueter le courriel sélectionné",
        }
    }

    pub fn label_selection_count(self, count: usize) -> String {
        match self {
            Self::English => format!("Label Selection ({count})"),
            Self::French => format!("Étiqueter la sélection ({count})"),
        }
    }

    pub fn btn_label_view(self) -> &'static str {
        match self {
            Self::English => "Label View",
            Self::French => "Étiqueter la vue",
        }
    }

    pub fn btn_select_unlabeled(self) -> &'static str {
        match self {
            Self::English => "Select Unlabeled",
            Self::French => "Sélectionner sans étiquette",
        }
    }

    // Selection action bar
    pub fn batch_label_label(self) -> &'static str {
        match self {
            Self::English => "Batch label:",
            Self::French => "Étiquetage groupé :",
        }
    }

    pub fn mails_selected_count(self, count: usize) -> String {
        match self {
            Self::English => format!("{count} mail{} selected", if count == 1 { "" } else { "s" }),
            Self::French => format!(
                "{count} courriel{} sélectionné{}",
                if count == 1 { "" } else { "s" },
                if count == 1 { "" } else { "s" }
            ),
        }
    }

    pub fn label_count_with_laya(self, count: usize) -> String {
        match self {
            Self::English => format!("Label {count} with Laya"),
            Self::French => format!("Étiqueter {count} avec Laya"),
        }
    }

    pub fn label_count_with_llm(self, count: usize) -> String {
        match self {
            Self::English => format!("Label {count} with LLM Judge"),
            Self::French => format!("Étiqueter {count} avec Juge LLM"),
        }
    }

    pub fn btn_clear_selection(self) -> &'static str {
        match self {
            Self::English => "Clear Selection",
            Self::French => "Effacer la sélection",
        }
    }

    pub fn remove_label_chip(self, name: &str) -> String {
        match self {
            Self::English => format!("Remove {name}"),
            Self::French => format!("Retirer {name}"),
        }
    }

    pub fn add_label_chip(self, name: &str) -> String {
        match self {
            Self::English => format!("Add {name}"),
            Self::French => format!("Ajouter {name}"),
        }
    }

    // Explorer - Table headers
    pub fn table_header_date_newest(self) -> &'static str {
        match self {
            Self::English => "DATE (Newest)",
            Self::French => "DATE (Plus récent)",
        }
    }

    pub fn table_header_date_oldest(self) -> &'static str {
        match self {
            Self::English => "DATE (Oldest)",
            Self::French => "DATE (Plus ancien)",
        }
    }

    pub fn table_header_date(self) -> &'static str {
        match self {
            Self::English => "DATE",
            Self::French => "DATE",
        }
    }

    pub fn table_header_subj_az(self) -> &'static str {
        match self {
            Self::English => "SUBJECT (A-Z)",
            Self::French => "SUJET (A-Z)",
        }
    }

    pub fn table_header_subj(self) -> &'static str {
        match self {
            Self::English => "SUBJECT",
            Self::French => "SUJET",
        }
    }

    pub fn table_header_from_az(self) -> &'static str {
        match self {
            Self::English => "CORRESPONDENT (A-Z)",
            Self::French => "CORRESPONDANT (A-Z)",
        }
    }

    pub fn table_header_from(self) -> &'static str {
        match self {
            Self::English => "CORRESPONDENT",
            Self::French => "CORRESPONDANT",
        }
    }

    pub fn table_header_labels(self) -> &'static str {
        match self {
            Self::English => "LABELS",
            Self::French => "ÉTIQUETTES",
        }
    }

    pub fn table_header_folder(self) -> &'static str {
        match self {
            Self::English => "FOLDER",
            Self::French => "DOSSIER",
        }
    }

    pub fn no_subject(self) -> &'static str {
        match self {
            Self::English => "(No subject)",
            Self::French => "(Sans sujet)",
        }
    }

    pub fn unknown_sender(self) -> &'static str {
        match self {
            Self::English => "(Unknown sender)",
            Self::French => "(Expéditeur inconnu)",
        }
    }

    pub fn root_folder(self) -> &'static str {
        match self {
            Self::English => "Root",
            Self::French => "Racine",
        }
    }

    pub fn unknown_date(self) -> &'static str {
        match self {
            Self::English => "Unknown",
            Self::French => "Inconnue",
        }
    }

    pub fn none_label(self) -> &'static str {
        match self {
            Self::English => "None",
            Self::French => "Aucune",
        }
    }

    pub fn empty_table_no_indexed(self) -> &'static str {
        match self {
            Self::English => {
                "No emails indexed yet. Choose a .pst archive or .jsonl fixture in the left pane and click 'Index now'."
            }
            Self::French => {
                "Aucun courriel encore indexé. Choisissez une archive .pst ou un fichier de test .jsonl dans le panneau de gauche et cliquez sur « Indexer »."
            }
        }
    }

    pub fn empty_table_no_match(self) -> &'static str {
        match self {
            Self::English => {
                "No messages match the current folder, label, or quick-filter selection."
            }
            Self::French => {
                "Aucun message ne correspond au dossier, à l'étiquette ou au filtre rapide sélectionné."
            }
        }
    }

    // Explorer - Reading pane
    pub fn no_message_selected(self) -> &'static str {
        match self {
            Self::English => "No message selected",
            Self::French => "Aucun message sélectionné",
        }
    }

    pub fn no_message_selected_hint(self) -> &'static str {
        match self {
            Self::English => {
                "Select a message in the table above to read its content, or check multiple messages to label a selection."
            }
            Self::French => {
                "Sélectionnez un message dans le tableau ci-dessus pour lire son contenu, ou cochez plusieurs messages pour étiqueter une sélection."
            }
        }
    }

    pub fn labels_heading(self) -> &'static str {
        match self {
            Self::English => "Labels:",
            Self::French => "Étiquettes :",
        }
    }

    pub fn none_assigned(self) -> &'static str {
        match self {
            Self::English => "None assigned",
            Self::French => "Aucune attribuée",
        }
    }

    pub fn toggle_heading(self) -> &'static str {
        match self {
            Self::English => "Toggle:",
            Self::French => "Basculer :",
        }
    }

    pub fn btn_label_with_laya(self) -> &'static str {
        match self {
            Self::English => "Label with Laya",
            Self::French => "Étiqueter avec Laya",
        }
    }

    pub fn btn_label_with_llm(self) -> &'static str {
        match self {
            Self::English => "Label with LLM Judge",
            Self::French => "Étiqueter avec Juge LLM",
        }
    }

    pub fn btn_ask_about_mail(self) -> &'static str {
        match self {
            Self::English => "Ask Chat about this mail",
            Self::French => "Interroger le chat sur ce courriel",
        }
    }

    pub fn from_prefix(self, from: &str) -> String {
        match self {
            Self::English => format!("From: {from}"),
            Self::French => format!("De : {from}"),
        }
    }

    pub fn to_prefix(self, to: &str) -> String {
        match self {
            Self::English => format!("To: {to}"),
            Self::French => format!("À : {to}"),
        }
    }

    pub fn unknown_date_full(self) -> &'static str {
        match self {
            Self::English => "Unknown date",
            Self::French => "Date inconnue",
        }
    }

    pub fn empty_body(self) -> &'static str {
        match self {
            Self::English => "(Empty message body)",
            Self::French => "(Corps du message vide)",
        }
    }

    // Chat
    pub fn chat_title(self) -> &'static str {
        match self {
            Self::English => "Archive Chat",
            Self::French => "Chat de l'archive",
        }
    }

    pub fn chat_subtitle(self) -> &'static str {
        match self {
            Self::English => "Grounded RAG assistant - Markdown enabled",
            Self::French => "Assistant RAG sourcé - Markdown activé",
        }
    }

    pub fn chat_welcome_title(self) -> &'static str {
        match self {
            Self::English => "Ask your archive",
            Self::French => "Interrogez votre archive",
        }
    }

    pub fn chat_welcome_desc(self) -> &'static str {
        match self {
            Self::English => {
                "Ask about decisions, people, dates, or projects. Only selected evidence is sent to your configured LLM."
            }
            Self::French => {
                "Posez des questions sur les décisions, les personnes, les dates ou les projets. Seules les preuves sélectionnées sont envoyées à votre LLM configuré."
            }
        }
    }

    pub fn prompt_summary_label(self) -> &'static str {
        match self {
            Self::English => "Summarize the latest decisions",
            Self::French => "Résumer les dernières décisions",
        }
    }

    pub fn prompt_summary_query(self) -> &'static str {
        match self {
            Self::English => "Summarize the latest decisions in my archive",
            Self::French => "Résume les dernières décisions dans mon archive",
        }
    }

    pub fn prompt_project_label(self) -> &'static str {
        match self {
            Self::English => "Find messages about a project",
            Self::French => "Trouver les messages sur un projet",
        }
    }

    pub fn prompt_project_query(self) -> &'static str {
        match self {
            Self::English => "Find the latest messages about my current projects",
            Self::French => "Trouve les messages récents concernant mes projets en cours",
        }
    }

    pub fn prompt_followup_label(self) -> &'static str {
        match self {
            Self::English => "What needs a follow-up?",
            Self::French => "Que faut-il relancer ?",
        }
    }

    pub fn prompt_followup_query(self) -> &'static str {
        match self {
            Self::English => "Which recent messages need a follow-up?",
            Self::French => "Quels messages récents nécessitent un suivi ou une relance ?",
        }
    }

    pub fn chat_input_placeholder(self) -> &'static str {
        match self {
            Self::English => "Ask your archive...",
            Self::French => "Interrogez votre archive...",
        }
    }

    pub fn chat_helper_need_index(self) -> &'static str {
        match self {
            Self::English => "Index an archive to start chatting",
            Self::French => "Indexez une archive pour commencer à discuter",
        }
    }

    pub fn chat_helper_busy(self) -> &'static str {
        match self {
            Self::English => "Working with your local archive...",
            Self::French => "Traitement de votre archive locale en cours...",
        }
    }

    pub fn chat_helper_ready(self) -> &'static str {
        match self {
            Self::English => "Enter to send - Search only stays local",
            Self::French => "Entrée pour envoyer - La recherche seule reste locale",
        }
    }

    pub fn btn_search_only(self) -> &'static str {
        match self {
            Self::English => "Search only",
            Self::French => "Recherche seule",
        }
    }

    pub fn btn_send(self) -> &'static str {
        match self {
            Self::English => "Send",
            Self::French => "Envoyer",
        }
    }

    pub fn evidence_heading(self) -> &'static str {
        match self {
            Self::English => "EVIDENCE",
            Self::French => "SOURCES",
        }
    }

    pub fn btn_hide_evidence(self) -> &'static str {
        match self {
            Self::English => "Hide evidence",
            Self::French => "Masquer les sources",
        }
    }

    pub fn btn_show_evidence(self) -> &'static str {
        match self {
            Self::English => "Show evidence",
            Self::French => "Afficher les sources",
        }
    }

    pub fn btn_open_mail(self) -> &'static str {
        match self {
            Self::English => "Open Mail",
            Self::French => "Ouvrir le courriel",
        }
    }

    pub fn btn_hide_details(self) -> &'static str {
        match self {
            Self::English => "Hide details",
            Self::French => "Masquer les détails",
        }
    }

    pub fn btn_metadata_and_excerpt(self) -> &'static str {
        match self {
            Self::English => "Metadata and full excerpt",
            Self::French => "Métadonnées et extrait complet",
        }
    }

    pub fn attachments_prefix(self, attachments: &str) -> String {
        match self {
            Self::English => format!("Attachments: {attachments}"),
            Self::French => format!("Pièces jointes : {attachments}"),
        }
    }

    pub fn labels_prefix(self, labels: &str) -> String {
        match self {
            Self::English => format!("Labels: {labels}"),
            Self::French => format!("Étiquettes : {labels}"),
        }
    }

    pub fn user_badge(self) -> &'static str {
        match self {
            Self::English => "YOU",
            Self::French => "VOUS",
        }
    }

    pub fn assistant_badge(self) -> &'static str {
        "ASSISTANT"
    }

    pub fn show_context_checkbox(self) -> &'static str {
        match self {
            Self::English => "Show context sent to LLM",
            Self::French => "Afficher le contexte envoyé au LLM",
        }
    }

    // Backend names
    pub fn backend_name(self, backend: LabelerBackend) -> &'static str {
        match (self, backend) {
            (Self::English, LabelerBackend::Laya) => "Laya (Local)",
            (Self::French, LabelerBackend::Laya) => "Laya (local)",
            (Self::English, LabelerBackend::LlmJudge) => "LLM Judge",
            (Self::French, LabelerBackend::LlmJudge) => "Juge LLM",
        }
    }

    // Status notifications
    pub fn status_storage_changed(self, path: &str) -> String {
        match self {
            Self::English => format!("Storage directory changed to {path}"),
            Self::French => format!("Dossier de stockage modifié pour {path}"),
        }
    }

    pub fn status_storage_saved_err(self, err: &str) -> String {
        match self {
            Self::English => format!(
                "Storage changed for this session, but the selection could not be saved: {err}"
            ),
            Self::French => format!(
                "Stockage modifié pour cette session, mais le choix n'a pas pu être enregistré : {err}"
            ),
        }
    }

    pub fn status_settings_saved(self, path: &str) -> String {
        match self {
            Self::English => format!("Settings saved in {path}"),
            Self::French => format!("Paramètres enregistrés dans {path}"),
        }
    }

    pub fn status_settings_save_err(self, err: &str) -> String {
        match self {
            Self::English => format!("Could not save settings: {err}"),
            Self::French => format!("Impossible d'enregistrer les paramètres : {err}"),
        }
    }

    pub fn status_language_changed(self) -> &'static str {
        match self {
            Self::English => "Language set to English",
            Self::French => "Langue définie sur Français",
        }
    }

    pub fn status_pst_enabled(self) -> &'static str {
        match self {
            Self::English => "PST enabled for retrieval",
            Self::French => "PST activé pour la recherche",
        }
    }

    pub fn status_pst_excluded(self) -> &'static str {
        match self {
            Self::English => "PST excluded from retrieval",
            Self::French => "PST exclu de la recherche",
        }
    }

    pub fn status_folders_enabled(self) -> &'static str {
        match self {
            Self::English => "Folder tree enabled for retrieval",
            Self::French => "Arborescence activée pour la recherche",
        }
    }

    pub fn status_folders_excluded(self) -> &'static str {
        match self {
            Self::English => "Folder tree excluded from retrieval",
            Self::French => "Arborescence exclue de la recherche",
        }
    }

    pub fn status_pst_removed(self, count: usize) -> String {
        match self {
            Self::English => format!("Removed {count} indexed messages and their embeddings"),
            Self::French => {
                format!("{count} messages indexés et leurs embeddings ont été supprimés")
            }
        }
    }

    pub fn status_pst_remove_err(self, err: &str) -> String {
        match self {
            Self::English => format!("Could not remove indexed PST: {err}"),
            Self::French => format!("Impossible de supprimer le PST indexé : {err}"),
        }
    }

    pub fn status_selected_visible(self) -> &'static str {
        match self {
            Self::English => "Selected visible emails",
            Self::French => "Courriels visibles sélectionnés",
        }
    }

    pub fn status_deselected_visible(self) -> &'static str {
        match self {
            Self::English => "Deselected visible emails",
            Self::French => "Courriels visibles désélectionnés",
        }
    }

    pub fn status_selected_unlabeled(self, added: usize, total: usize) -> String {
        match self {
            Self::English => format!(
                "Selected {added} unlabeled email{} in current view ({total} total selected)",
                if added == 1 { "" } else { "s" }
            ),
            Self::French => format!(
                "{added} courriel{} sans étiquette sélectionné{} dans la vue actuelle ({total} au total)",
                if added == 1 { "" } else { "s" },
                if added == 1 { "" } else { "s" }
            ),
        }
    }

    pub fn status_selected_range(self, count: usize, total: usize) -> String {
        match self {
            Self::English => format!("Selected range of {count} emails ({total} total selected)"),
            Self::French => format!("Plage de {count} courriels sélectionnée ({total} au total)"),
        }
    }

    pub fn status_labeling_select_first(self) -> &'static str {
        match self {
            Self::English => "Select an email in the explorer first",
            Self::French => "Sélectionnez d'abord un courriel dans l'explorateur",
        }
    }

    pub fn status_labeling_no_matches(self) -> &'static str {
        match self {
            Self::English => "No matching emails found for selection",
            Self::French => "Aucun courriel correspondant trouvé pour la sélection",
        }
    }

    pub fn status_labeling_load_err(self, err: &str) -> String {
        match self {
            Self::English => format!("Could not load selected emails: {err}"),
            Self::French => format!("Impossible de charger les courriels sélectionnés : {err}"),
        }
    }

    pub fn status_labeling_no_visible(self) -> &'static str {
        match self {
            Self::English => "No messages in the current explorer view to label",
            Self::French => "Aucun message à étiqueter dans la vue actuelle",
        }
    }

    pub fn status_selected_visible_count(self, count: usize) -> String {
        match self {
            Self::English => format!(
                "Selected {count} visible email{}",
                if count == 1 { "" } else { "s" }
            ),
            Self::French => format!(
                "{count} courriel{} visible{} sélectionné{}",
                if count == 1 { "" } else { "s" },
                if count == 1 { "" } else { "s" },
                if count == 1 { "" } else { "s" }
            ),
        }
    }

    pub fn status_cleared_selection(self) -> &'static str {
        match self {
            Self::English => "Cleared mail selection",
            Self::French => "Sélection de courriels effacée",
        }
    }

    pub fn status_select_archive_first(self) -> &'static str {
        match self {
            Self::English => "Select an archive first",
            Self::French => "Sélectionnez d'abord une archive",
        }
    }

    pub fn status_enter_embedding_model_first(self) -> &'static str {
        match self {
            Self::English => "Enter an embedding model before enabling remote embeddings",
            Self::French => {
                "Indiquez un modèle d'embedding avant d'activer les embeddings distants"
            }
        }
    }

    pub fn status_enter_question_first(self) -> &'static str {
        match self {
            Self::English => "Enter a question first",
            Self::French => "Saisissez d'abord une question",
        }
    }

    pub fn status_searching_local_index(self) -> &'static str {
        match self {
            Self::English => "Searching local index...",
            Self::French => "Recherche dans l'index local...",
        }
    }

    pub fn status_prepared_prompt_email(self) -> &'static str {
        match self {
            Self::English => "Prepared prompt for the selected email in the Chat pane",
            Self::French => "Invite préparée pour le courriel sélectionné dans le volet de chat",
        }
    }

    pub fn status_enter_search_terms_first(self) -> &'static str {
        match self {
            Self::English => "Enter search terms first",
            Self::French => "Saisissez d'abord des termes de recherche",
        }
    }

    pub fn status_searching_locally(self) -> &'static str {
        match self {
            Self::English => "Searching locally - no network request...",
            Self::French => "Recherche locale en cours - aucune requête réseau...",
        }
    }

    pub fn status_new_conversation(self) -> &'static str {
        match self {
            Self::English => "New conversation - the local index is still available",
            Self::French => "Nouvelle conversation - l'index local reste disponible",
        }
    }

    pub fn status_selected_laya_gguf(self, path: &str) -> String {
        match self {
            Self::English => format!("Selected Laya GGUF weights at {path}"),
            Self::French => format!("Poids GGUF Laya sélectionnés à l'emplacement {path}"),
        }
    }

    pub fn status_downloading_laya(self, path: &str) -> String {
        match self {
            Self::English => format!("Downloading ggml-org/Laya-GGUF to {path}..."),
            Self::French => format!("Téléchargement de ggml-org/Laya-GGUF vers {path}..."),
        }
    }

    pub fn status_label_updated(self, label: &str, added: bool) -> String {
        match self {
            Self::English => format!(
                "{} label '{label}'",
                if added { "Added" } else { "Removed" }
            ),
            Self::French => format!(
                "Étiquette « {label} » {}",
                if added { "ajoutée" } else { "retirée" }
            ),
        }
    }

    pub fn status_check_emails_first(self) -> &'static str {
        match self {
            Self::English => "Check one or more emails first",
            Self::French => "Cochez d'abord un ou plusieurs courriels",
        }
    }

    pub fn status_label_batch_updated(self, label: &str, assign: bool, count: usize) -> String {
        match self {
            Self::English => format!(
                "{} label '{label}' for {count} email{}",
                if assign { "Added" } else { "Removed" },
                if count == 1 { "" } else { "s" }
            ),
            Self::French => format!(
                "Étiquette « {label} » {} pour {count} courriel{}",
                if assign { "ajoutée" } else { "retirée" },
                if count == 1 { "" } else { "s" }
            ),
        }
    }

    pub fn status_enter_label_name_first(self) -> &'static str {
        match self {
            Self::English => "Enter a label name first",
            Self::French => "Saisissez d'abord un nom d'étiquette",
        }
    }

    pub fn status_saved_label(self, name: &str) -> String {
        match self {
            Self::English => format!("Saved label '{name}'"),
            Self::French => format!("Étiquette « {name} » enregistrée"),
        }
    }

    pub fn status_removed_label(self, name: &str) -> String {
        match self {
            Self::English => format!("Removed label '{name}'"),
            Self::French => format!("Étiquette « {name} » supprimée"),
        }
    }

    pub fn status_labeling_emails_with(self, total: usize, backend: &str) -> String {
        match self {
            Self::English => format!(
                "Labeling {total} email{} with {backend}...",
                if total == 1 { "" } else { "s" }
            ),
            Self::French => format!(
                "Étiquetage de {total} courriel{} avec {backend}...",
                if total == 1 { "" } else { "s" }
            ),
        }
    }

    pub fn status_labeling_progress(self, idx: usize, total: usize, backend: &str) -> String {
        match self {
            Self::English => format!("Labeling email {idx} of {total} with {backend}..."),
            Self::French => format!("Étiquetage du courriel {idx} sur {total} avec {backend}..."),
        }
    }

    pub fn status_label_update_err(self, label: &str, err: &str) -> String {
        match self {
            Self::English => format!("Could not update label '{label}': {err}"),
            Self::French => format!("Impossible de modifier l'étiquette « {label} » : {err}"),
        }
    }

    pub fn status_label_batch_err(self, label: &str, err: &str) -> String {
        match self {
            Self::English => format!("Could not update label '{label}' on selection: {err}"),
            Self::French => {
                format!("Impossible de modifier l'étiquette « {label} » sur la sélection : {err}")
            }
        }
    }

    pub fn status_label_save_err(self, name: &str, err: &str) -> String {
        match self {
            Self::English => format!("Could not save label '{name}': {err}"),
            Self::French => format!("Impossible d'enregistrer l'étiquette « {name} » : {err}"),
        }
    }

    pub fn status_label_remove_err(self, name: &str, err: &str) -> String {
        match self {
            Self::English => format!("Could not remove label '{name}': {err}"),
            Self::French => format!("Impossible de supprimer l'étiquette « {name} » : {err}"),
        }
    }

    pub fn worker_search_found(self, count: usize) -> String {
        match self {
            Self::English => format!(
                "I found {count} matching email{} in your local archive. The sources are shown below.",
                if count == 1 { "" } else { "s" }
            ),
            Self::French => format!(
                "J'ai trouvé {count} courriel{} correspondant{} dans votre archive locale. Les sources sont affichées ci-dessous.",
                if count == 1 { "" } else { "s" },
                if count == 1 { "" } else { "s" }
            ),
        }
    }

    pub fn worker_search_status(self, count: usize) -> String {
        match self {
            Self::English => {
                format!("Found {count} local results - nothing was sent over the network")
            }
            Self::French => format!(
                "{count} résultat{} locaux trouvés - rien n'a été envoyé sur le réseau",
                if count <= 1 { "" } else { "s" }
            ),
        }
    }

    pub fn worker_search_failed(self, error: &str) -> String {
        match self {
            Self::English => format!("I could not complete that local search: {error}"),
            Self::French => format!("Impossible d'effectuer cette recherche locale : {error}"),
        }
    }

    pub fn worker_answer_received(self) -> &'static str {
        match self {
            Self::English => "Answer received",
            Self::French => "Réponse reçue",
        }
    }

    pub fn worker_answer_failed(self, error: &str) -> String {
        match self {
            Self::English => format!("I could not answer that question: {error}"),
            Self::French => format!("Impossible de répondre à cette question : {error}"),
        }
    }

    pub fn worker_indexed(self, added: usize, embedded: usize) -> String {
        match self {
            Self::English => {
                if embedded > 0 {
                    format!(
                        "Indexed {added} new messages with BM25 and stored {embedded} remote embeddings"
                    )
                } else {
                    format!("Indexed {added} new messages with local BM25")
                }
            }
            Self::French => {
                if embedded > 0 {
                    format!(
                        "{added} nouveaux messages indexés avec BM25 et {embedded} embeddings distants enregistrés"
                    )
                } else {
                    format!("{added} nouveaux messages indexés avec BM25 local")
                }
            }
        }
    }

    pub fn worker_labeled(
        self,
        cancelled: bool,
        messages: usize,
        labels: usize,
        source: &str,
    ) -> String {
        match self {
            Self::English => {
                let prefix = if cancelled {
                    "Stopped labeling after"
                } else {
                    "Labeled"
                };
                format!(
                    "{prefix} {messages} email{} ({labels} label{} assigned via {source})",
                    if messages == 1 { "" } else { "s" },
                    if labels == 1 { "" } else { "s" },
                )
            }
            Self::French => {
                let prefix = if cancelled {
                    "Étiquetage interrompu après"
                } else {
                    "Étiquetage terminé pour"
                };
                format!(
                    "{prefix} {messages} courriel{} ({labels} étiquette{} attribuée{} via {source})",
                    if messages == 1 { "" } else { "s" },
                    if labels == 1 { "" } else { "s" },
                    if labels == 1 { "" } else { "s" },
                )
            }
        }
    }

    pub fn worker_labeling_failed(self, error: &str) -> String {
        match self {
            Self::English => format!("Labeling failed: {error}"),
            Self::French => format!("Échec de l'étiquetage : {error}"),
        }
    }

    pub fn worker_laya_downloaded(self, path: &str) -> String {
        match self {
            Self::English => format!("Downloaded ggml-org/Laya-GGUF to {path}"),
            Self::French => format!("Modèle ggml-org/Laya-GGUF téléchargé vers {path}"),
        }
    }

    pub fn agent_progress_search(self, query: &str) -> String {
        match self {
            Self::English => format!("Searching archive: {query}..."),
            Self::French => format!("Recherche dans l'archive : {query}..."),
        }
    }

    pub fn agent_progress_read(self, id: i64) -> String {
        match self {
            Self::English => format!("Reading email #{id}..."),
            Self::French => format!("Lecture du courriel #{id}..."),
        }
    }

    pub fn agent_progress_thread(self, count: usize) -> String {
        match self {
            Self::English => format!(
                "Reading thread ({count} message{})...",
                if count == 1 { "" } else { "s" }
            ),
            Self::French => format!(
                "Lecture du fil ({count} message{})...",
                if count == 1 { "" } else { "s" }
            ),
        }
    }

    pub fn agent_progress_stats(self, group: &str) -> String {
        match self {
            Self::English => format!("Calculating statistics (group: {group})..."),
            Self::French => format!("Calcul des statistiques (groupe : {group})..."),
        }
    }

    pub fn agent_progress_thinking(self) -> &'static str {
        match self {
            Self::English => "Investigating archive...",
            Self::French => "Exploration de l'archive...",
        }
    }

    pub fn agent_progress_finalizing(self) -> &'static str {
        match self {
            Self::English => "Synthesizing answer from collected evidence...",
            Self::French => "Synthèse de la réponse à partir des preuves collectées...",
        }
    }

    pub fn agent_today_label(self) -> &'static str {
        match self {
            Self::English => "Today's reference date:",
            Self::French => "Date de référence du jour :",
        }
    }

    pub fn agent_cancelled(self) -> &'static str {
        match self {
            Self::English => "Agent operation cancelled by user.",
            Self::French => "Opération de l'agent annulée par l'utilisateur.",
        }
    }

    pub fn agent_budget_exhausted_note(self) -> &'static str {
        match self {
            Self::English => {
                "Note: Research turn limit reached. Answer using the evidence gathered so far."
            }
            Self::French => {
                "Note : Limite d'étapes de recherche atteinte. Répondez à l'aide des preuves collectées jusqu'ici."
            }
        }
    }

    pub fn agent_no_tool_calls(self) -> &'static str {
        match self {
            Self::English => "Answered directly without extra tool calls.",
            Self::French => "Réponse directe sans appel d'outil supplémentaire.",
        }
    }

    pub fn agent_trace_heading(self) -> &'static str {
        match self {
            Self::English => "### Research Trace",
            Self::French => "### Journal de recherche",
        }
    }
}
