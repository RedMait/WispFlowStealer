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
    // Back-to-back punctuation from the recognizer stays glued.
    assert_eq!(ru("привет., мир"), "Привет., мир.");
    assert_eq!(
        ru("выручка выросла на 12%, но упала"),
        "Выручка выросла на 12%, но упала."
    );
}

#[test]
fn whitespace_and_double_spaces_are_normalized() {
    assert_eq!(ru("привет    мир"), "Привет мир.");
    assert_eq!(ru("  я   иду   домой  "), "Я иду домой.");
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
