use crate::{
    i18n::Language,
    llm::{ChatTurn, LlmConfig, agent_system_prompt, contextualize_search_query},
};

#[test]
fn language_defaults_to_french() {
    assert_eq!(Language::default(), Language::French);
}

#[test]
fn config_serializes_and_deserializes_with_language() {
    let config = LlmConfig {
        endpoint: "https://api.openai.com/v1".to_string(),
        model: "gpt-4o-mini".to_string(),
        use_remote_embeddings: false,
        embedding_model: "text-embedding-3-small".to_string(),
        bearer_token: "sk-test".to_string(),
        max_context_chars: 24_000,
        labeler_backend: crate::model::LabelerBackend::Laya,
        laya_gguf_path: String::new(),
        language: Language::French,
    };

    let serialized = serde_json::to_string(&config).expect("serialize config");
    assert!(serialized.contains(r#""language":"French""#));

    let deserialized: LlmConfig = serde_json::from_str(&serialized).expect("deserialize config");
    assert_eq!(deserialized.language, Language::French);

    // Backward compatibility: when "language" is missing in older json, it defaults to French
    let legacy_json = r#"{
        "endpoint": "https://api.openai.com/v1",
        "model": "gpt-4o-mini",
        "embedding_model": "text-embedding-3-small",
        "bearer_token": "sk-test",
        "max_context_chars": 24000,
        "labeler_backend": "Laya"
    }"#;
    let legacy_config: LlmConfig = serde_json::from_str(legacy_json).expect("deserialize legacy config");
    assert_eq!(legacy_config.language, Language::French);
}

#[test]
fn system_prompts_match_language() {
    let fr_prompt = agent_system_prompt(Language::French);
    assert!(fr_prompt.contains("Vous êtes Dungbeetle"));
    assert!(fr_prompt.contains("courriels"));
    assert!(fr_prompt.contains("[Email 1]"));

    let en_prompt = agent_system_prompt(Language::English);
    assert!(en_prompt.contains("You are Dungbeetle"));
    assert!(en_prompt.contains("[Email 1]"));
}

#[test]
fn french_ui_translations_are_complete_and_accurate() {
    let fr = Language::French;
    assert_eq!(fr.app_title(), "PSTbeetle - Explorateur de courriels et RAG local");
    assert_eq!(fr.section_mailboxes(), "BOÎTES ET DOSSIERS");
    assert_eq!(fr.section_labels(), "ÉTIQUETTES DE COURRIEL");
    assert_eq!(fr.section_laya(), "MODÈLE DE DÉCISION LAYA");
    assert_eq!(fr.section_ai(), "JUGE LLM ET RAG");
    assert_eq!(fr.table_header_subj(), "SUJET");
    assert_eq!(fr.table_header_from(), "CORRESPONDANT");
    assert_eq!(fr.table_header_date(), "DATE");
    assert_eq!(fr.chat_title(), "Chat de l'archive");
    assert_eq!(fr.chat_welcome_title(), "Interrogez votre archive");
    assert_eq!(fr.no_message_selected(), "Aucun message sélectionné");

    // Dynamic formatting tests
    assert_eq!(
        fr.worker_labeled(false, 3, 2, "Laya"),
        "Étiquetage terminé pour 3 courriels (2 étiquettes attribuées via Laya)"
    );
    assert_eq!(
        fr.worker_labeled(true, 1, 1, "Laya"),
        "Étiquetage interrompu après 1 courriel (1 étiquette attribuée via Laya)"
    );
    assert_eq!(
        fr.status_showing(10, 50, 2, "Laya"),
        "Affichage de 10 sur 50 indexés, 2 sélectionné(s), Étiqueteur : Laya"
    );
}

#[test]
fn contextualizes_french_query_with_history() {
    let history = vec![
        ChatTurn {
            role: "user".to_string(),
            content: "Qui est responsable de la facture d'avril ?".to_string(),
        },
        ChatTurn {
            role: "assistant".to_string(),
            content: "Jean Dupont est en charge selon [Email 1].".to_string(),
        },
    ];

    let contextualized = contextualize_search_query("qu'en est-il de son adresse ?", &history);
    assert!(contextualized.to_lowercase().contains("facture"));
}
