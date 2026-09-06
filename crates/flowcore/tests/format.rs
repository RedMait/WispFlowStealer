// SPDX-License-Identifier: MIT
use flowcore::{classify, format, format_raw, Language, SentenceKind};

fn ru(raw: &str) -> String {
    format(raw, Language::Ru)
}

fn en(raw: &str) -> String {
    format(raw, Language::En)
}

#[test]
fn language_detection() {
    assert_eq!(Language::detect("привет мир"), Language::Ru);
    assert_eq!(Language::detect("ёлка"), Language::Ru);
    assert_eq!(Language::detect("hello world"), Language::En);
    assert_eq!(Language::detect("«зима» и snow"), Language::Ru);
}

#[test]
fn empty_input_is_empty() {
    assert_eq!(ru(""), "");
    assert_eq!(ru("   "), "");
    assert_eq!(en(""), "");
}

#[test]
fn ru_simple_statement_gets_capitalized_and_punctuated() {
    assert_eq!(ru("привет мир"), "Привет мир.");
}

#[test]
fn en_simple_statement_gets_capitalized_and_punctuated() {
    assert_eq!(en("hello world"), "Hello world.");
}

#[test]
fn ru_single_filler_word_is_removed() {
    assert_eq!(ru("ну привет мир"), "Привет мир.");
    assert_eq!(ru("привет типа мир"), "Привет мир.");
    assert_eq!(ru("вот так вот работает"), "Вот так вот работает.");
}

#[test]
fn ru_multiword_fillers_are_removed() {
    assert_eq!(ru("в общем я иду домой"), "Я иду домой.");
    assert_eq!(ru("как бы это лучше сделать"), "Это лучше сделать.");
    assert_eq!(ru("ну вот и и всё"), "И всё.");
}

#[test]
fn en_single_filler_words_are_removed() {
    assert_eq!(en("um hello world"), "Hello world.");
    assert_eq!(en("like this is um great"), "This is great!");
}

#[test]
fn en_multiword_fillers_are_removed() {
    assert_eq!(en("so basically you know i mean it works"), "It works.");
    assert_eq!(en("that is kind of a nice idea"), "That is a nice idea.");
}

#[test]
fn repeated_words_are_collapsed() {
    assert_eq!(ru("и и я я вышел вышел"), "И я вышел.");
    assert_eq!(en("the the cat cat is fine"), "The cat is fine.");
}

#[test]
fn ru_question_detection() {
    assert_eq!(
        classify("почему так вышло", Language::Ru),
        SentenceKind::Question
    );
    assert_eq!(ru("почему так вышло"), "Почему так вышло?");
    assert_eq!(ru("что ты делаешь"), "Что ты делаешь?");
    assert_eq!(ru("где мой телефон"), "Где мой телефон?");
    assert_eq!(ru("можно мне чаю"), "Можно мне чаю?");
}

#[test]
fn en_question_detection() {
    assert_eq!(en("what are you doing"), "What are you doing?");
    assert_eq!(en("is it ready"), "Is it ready?");
    assert_eq!(en("can we leave now"), "Can we leave now?");
    assert_eq!(en("how does this work"), "How does this work?");
}

#[test]
fn ru_exclamation_detection() {
    assert_eq!(ru("это просто потрясающе"), "Это просто потрясающе!");
    assert_eq!(ru("какой красивый кролик"), "Какой красивый кролик!");
    assert_eq!(ru("ура мы победили"), "Ура мы победили!");
}

#[test]
fn en_exclamation_detection() {
    assert_eq!(en("this is amazing"), "This is amazing!");
    assert_eq!(en("wow that was great"), "Wow that was great!");
    assert_eq!(en("what an incredible idea"), "What an incredible idea!");
}

#[test]
fn explicit_punctuation_wins() {
    assert_eq!(ru("привет мир?"), "Привет мир?");
    assert_eq!(ru("привет мир!"), "Привет мир!");
    assert_eq!(ru("привет мир."), "Привет мир.");
    assert_eq!(en("really?"), "Really?");
    assert_eq!(en("ouch!"), "Ouch!");
}

#[test]
fn commas_are_kept_and_spaced_correctly() {
    assert_eq!(
        ru("ну и я тут подумал ну что делать"),
        "Я тут подумал, что делать."
    );
    assert_eq!(ru("привет , мир"), "Привет, мир.");
}

#[test]
fn heuristic_commas_ru() {
    assert_eq!(ru("я подумал ну что делать"), "Я подумал, что делать.");
    assert_eq!(
        ru("я хочу пойти домой потому что устал"),
        "Я хочу пойти домой, потому что устал."
    );
    assert_eq!(
        ru("это невозможно например в такую погоду"),
        "Это невозможно, например, в такую погоду."
    );
    assert_eq!(ru("приходи когда хочешь"), "Приходи когда хочешь.");
    assert_eq!(ru("конечно же можно"), "Конечно же, можно.");
    assert_eq!(ru("но я всё равно пойду"), "Но я всё равно пойду.");
    assert_eq!(
        ru("я знаю человека который всё понимает"),
        "Я знаю человека, который всё понимает."
    );
}

#[test]
fn heuristic_commas_en() {
    assert_eq!(
        en("i stayed home because it rained"),
        "I stayed home, because it rained."
    );
    assert_eq!(en("it depends on who asks"), "It depends on who asks.");
    assert_eq!(en("but i will still go"), "But I will still go.");
}

#[test]
fn clean_returns_bare_lowercase_words() {
    assert_eq!(
        flowcore::clean("Ну привет мир! Это типа классно", Language::Ru),
        "привет мир это классно"
    );
    assert_eq!(
        flowcore::clean("um hello world. wow", Language::En),
        "hello world wow"
    );
}

#[test]
fn conjunction_a_survives_and_punct_glues() {
    // Bare "а" is a live conjunction, not a filler.
    assert_eq!(ru("а также пирожок"), "А также пирожок.");
    assert_eq!(
        ru("тоже хочу чаю, а также пирожок"),
        "Тоже хочу чаю, а также пирожок."
    );
    // Back-to-back punctuation from the recognizer: period wins, next
    // sentence gets its capital.
    assert_eq!(ru("привет., мир"), "Привет. Мир.");
    assert_eq!(
        ru("выручка выросла на 12%, но упала"),
        "Выручка выросла на 12%, но упала."
    );
}

#[test]
fn adjacent_marks_collapse_and_sentences_case() {
    // Recognizer glitches: comma+period stuck together.
    assert_eq!(ru("все готово., жду звонка"), "Все готово. Жду звонка.");
    assert_eq!(ru("все готово,. жду звонка"), "Все готово. Жду звонка.");
    assert_eq!(ru("привет,, мир"), "Привет, мир.");
    assert_eq!(ru("да!? серьезно"), "Да?! Серьезно.");
    // Internal sentence boundary gets its capital.
    assert_eq!(ru("все готово. жду звонка"), "Все готово. Жду звонка.");
    // Ellipsis and decimal numbers survive.
    assert_eq!(ru("ждал... и вот"), "Ждал... И вот.");
    assert_eq!(ru("версия 3.14 вышла"), "Версия 3.14 вышла.");
}

#[test]
fn profiles_shape_output() {
    use flowcore::Profile;
    assert_eq!(Profile::parse("chat"), Profile::Chat);
    assert_eq!(Profile::parse("ПОЧТА"), Profile::Mail);
    assert_eq!(Profile::parse("code"), Profile::Code);
    assert_eq!(Profile::parse(""), Profile::Auto);
    assert_eq!(Profile::resolve(Profile::Chat, "Telegram"), Profile::Chat);
    assert_eq!(
        Profile::resolve(Profile::Auto, "Telegram Desktop"),
        Profile::Chat
    );
    assert_eq!(
        Profile::resolve(Profile::Auto, "Inbox - Outlook"),
        Profile::Mail
    );
    assert_eq!(
        Profile::resolve(Profile::Auto, "main.rs - Visual Studio Code"),
        Profile::Code
    );
    assert_eq!(
        Profile::resolve(Profile::Auto, "Untitled Notepad"),
        Profile::Mail
    );
    assert_eq!(
        flowcore::format_code("удали  типа  вот этот  этот thunk"),
        "удали типа вот этот thunk"
    );
    assert_eq!(flowcore::format_code("  spaced   out  "), "spaced out");
    assert_eq!(flowcore::pad_replica_start("привет", true), " привет");
    assert_eq!(flowcore::pad_replica_start("привет", false), "привет");
    assert_eq!(flowcore::pad_replica_start(", привет", true), ", привет");
}

#[test]
fn language_resolve_prefers_explicit_choice() {
    use flowcore::{word_count, Language};
    assert_eq!(Language::resolve(Some("ru"), "hello world"), Language::Ru);
    assert_eq!(Language::resolve(Some("EN"), "привет мир"), Language::En);
    assert_eq!(Language::resolve(None, "привет мир"), Language::Ru);
    assert_eq!(Language::resolve(Some("auto"), "hello"), Language::En);
    assert_eq!(Language::resolve(Some("nonsense"), "привет"), Language::Ru);
    assert_eq!(word_count("  раз два  три "), 3);
    assert_eq!(word_count(""), 0);
    assert_eq!(flowcore::transliterate_ru("Привет, мир!"), "Privet, mir!");
    assert_eq!(flowcore::transliterate_ru("Щука Ёж"), "Shchuka Ezh");
    assert!(flowcore::meets_min_len("Я.", 2));
    assert!(flowcore::meets_min_len("раз два три", 2));
    assert!(!flowcore::meets_min_len(",", 2));
    assert!(!flowcore::meets_min_len("", 2));
}

#[test]
fn whitespace_and_double_spaces_are_normalized() {
    assert_eq!(ru("привет    мир"), "Привет мир.");
    assert_eq!(ru("  я   иду   домой  "), "Я иду домой.");
}

#[test]
fn normalizers_unify_numbers_dates_times() {
    use flowcore::FormatOpts;
    let words = FormatOpts {
        numbers_words: true,
    };
    let digits = FormatOpts::default();
    // F-10: digits -> words only when asked; attached runs survive.
    assert_eq!(
        flowcore::digits_to_words("купи 12 стульев", true),
        "купи двенадцать стульев"
    );
    assert_eq!(
        flowcore::digits_to_words("купи 12 стульев", false),
        "купи 12 стульев"
    );
    assert_eq!(
        flowcore::digits_to_words("COVID-19 и gpt-4", true),
        "COVID-19 и gpt-4"
    );
    assert_eq!(flowcore::digits_to_words("12.03", true), "12.03");
    assert_eq!(flowcore::num_to_ru_words(0), Some("ноль".to_string()));
    assert_eq!(flowcore::num_to_ru_words(1), Some("один".to_string()));
    assert_eq!(flowcore::num_to_ru_words(2), Some("два".to_string()));
    assert_eq!(
        flowcore::num_to_ru_words(21),
        Some("двадцать один".to_string())
    );
    assert_eq!(
        flowcore::num_to_ru_words(112),
        Some("сто двенадцать".to_string())
    );
    assert_eq!(
        flowcore::num_to_ru_words(2000),
        Some("две тысячи".to_string())
    );
    assert_eq!(
        flowcore::num_to_ru_words(60000),
        Some("шестьдесят тысяч".to_string())
    );
    // F-11: dates.
    assert_eq!(
        flowcore::normalize_dates("встреча 12 марта"),
        "встреча 12.03"
    );
    assert_eq!(
        flowcore::normalize_dates("встреча 12 марта 2024 года"),
        "встреча 12.03.2024 года"
    );
    assert_eq!(flowcore::normalize_dates("до 12/03"), "до 12.03");
    assert_eq!(flowcore::normalize_dates("уже 12.03"), "уже 12.03");
    // F-12: times.
    assert_eq!(flowcore::normalize_times("в 3 часа"), "в 3:00");
    assert_eq!(flowcore::normalize_times("5 часов 20 минут"), "5:20");
    assert_eq!(flowcore::normalize_times("в 7 вечера"), "в 19:00");
    assert_eq!(flowcore::normalize_times("в 15:30"), "в 15:30");
    // End to end through format_with.
    assert_eq!(
        flowcore::format_with("встреча 12 марта в 3 часа", flowcore::Language::Ru, words),
        "Встреча 12.03 в 3:00."
    );
    assert_eq!(
        flowcore::format_with("купи 12 стульев", flowcore::Language::Ru, digits),
        "Купи 12 стульев."
    );
}

#[test]
fn english_i_is_capitalized() {
    assert_eq!(en("i think i can do it"), "I think I can do it.");
}

#[test]
fn all_fillers_leaves_empty() {
    assert_eq!(ru("ну ну э-э мм"), "");
    assert_eq!(en("um uh hmm like"), "");
}

#[test]
fn apostrophes_survive() {
    assert_eq!(en("don't worry"), "Don't worry.");
    assert_eq!(en("it's a fine idea"), "It's a fine idea.");
}

#[test]
fn format_raw_detects_language() {
    assert_eq!(format_raw("привет мир"), "Привет мир.");
    assert_eq!(format_raw("hello world"), "Hello world.");
}

#[test]
fn no_double_punctuation() {
    assert_eq!(ru("что делать?"), "Что делать?");
    assert_eq!(ru("привет."), "Привет.");
    assert_eq!(en("really?!"), "Really?!");
}

#[test]
fn leading_punctuation_does_not_clutter() {
    assert_eq!(ru("... привет мир"), "Привет мир.");
    assert_eq!(ru(", привет"), "Привет.");
}

#[test]
fn sentence_kind_display() {
    assert_eq!(SentenceKind::Statement.punctuation(), '.');
    assert_eq!(SentenceKind::Question.punctuation(), '?');
    assert_eq!(SentenceKind::Exclamation.punctuation(), '!');
    assert_eq!(SentenceKind::Statement.to_string(), "statement");
    assert_eq!(SentenceKind::Question.to_string(), "question");
    assert_eq!(SentenceKind::Exclamation.to_string(), "exclamation");
    assert_eq!(Language::Ru.to_string(), "ru");
    assert_eq!(Language::En.to_string(), "en");
}
