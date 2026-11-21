#!/usr/bin/env python3
"""Build the explicitly draft 11-language coreference baseline matrix."""

from __future__ import annotations

import json
from pathlib import Path

OUTPUT = Path(__file__).with_name("coref_gold.json")

LANGUAGE_DATA = {
    "de": {
        "female": ["Anna Schmidt", "Maria Weber", "Lisa Fischer", "Emma Wagner", "Sophie Becker", "Laura Hoffmann", "Lena Schulz", "Julia Koch", "Mia Bauer", "Clara Wolf"],
        "male": ["Hans Müller", "Thomas Fischer", "Peter Weber", "Paul Schmidt", "Lukas Wagner", "Felix Becker", "Leon Hoffmann", "Max Schulz", "Jonas Koch", "Ben Bauer"],
        "singular": "Sie", "plural": "Sie",
        "unique": ["kam an", "kehrte zurück", "trat ein", "reiste ab", "blieb stehen", "setzte sich", "ging weiter", "wartete", "nickte", "rief an"],
        "pronoun_verb": ["lächelte", "winkte", "antwortete", "ging", "blieb", "sprach", "sang", "rannte", "wartete", "nickte"],
        "ambiguous": "{a} traf {b}, und {p} {v}.",
        "amb_verb": ["lächelte", "winkte", "antwortete", "ging", "blieb", "sprach", "sang", "rannte", "wartete", "nickte"],
        "unresolved": "{p} {v}.",
        "pair": "{a} und {b} kamen an. {p} feierten.",
        "male_verb": ["kamen an", "kehrten zurück", "traten ein", "reisten ab", "blieben stehen", "setzten sich", "gingen weiter", "warteten", "nickten", "riefen an"],
        "plural_verb": "feierten",
    },
    "en": {
        "female": ["Anna Smith", "Mary Jones", "Emma Brown", "Sarah Wilson", "Olivia Taylor", "Jane Miller", "Lily Davis", "Grace Martin", "Alice Moore", "Kate Lee"],
        "male": ["John Smith", "James Jones", "Peter Brown", "David Wilson", "Oliver Taylor", "Mark Miller", "Daniel Davis", "Henry Martin", "Michael Moore", "Robert Lee"],
        "singular": "She", "plural": "They",
        "unique": ["arrived", "returned", "entered", "left", "stopped", "sat down", "continued", "waited", "nodded", "called"],
        "pronoun_verb": ["smiled", "waved", "answered", "left", "stayed", "spoke", "sang", "ran", "waited", "nodded"],
        "ambiguous": "{a} met {b}, and {p} {v}.",
        "amb_verb": ["smiled", "waved", "answered", "left", "stayed", "spoke", "sang", "ran", "waited", "nodded"],
        "unresolved": "{p} {v}.",
        "pair": "{a} and {b} arrived. {p} celebrated.",
        "male_verb": ["arrived", "returned", "entered", "left", "stopped", "sat down", "continued", "waited", "nodded", "called"],
        "plural_verb": "celebrated",
    },
    "es": {
        "female": ["Ana García", "María López", "Lucía Pérez", "Carmen Sánchez", "Elena Martín", "Paula Ruiz", "Laura Torres", "Sofía Romero", "Marta Díaz", "Isabel Flores"],
        "male": ["Juan García", "José López", "Luis Pérez", "Carlos Sánchez", "Pedro Martín", "Pablo Ruiz", "Diego Torres", "Miguel Romero", "Javier Díaz", "Andrés Flores"],
        "singular": "Ella", "plural": "Ellos",
        "unique": ["llegó", "regresó", "entró", "salió", "se detuvo", "se sentó", "continuó", "esperó", "asintió", "llamó"],
        "pronoun_verb": ["sonrió", "saludó", "respondió", "se fue", "se quedó", "habló", "cantó", "corrió", "esperó", "asintió"],
        "ambiguous": "{a} vio a {b}, y {p} {v}.",
        "amb_verb": ["sonrió", "saludó", "respondió", "se fue", "se quedó", "habló", "cantó", "corrió", "esperó", "asintió"],
        "unresolved": "{p} {v}.",
        "pair": "{a} y {b} llegaron. {p} celebraron.",
        "male_verb": ["llegaron", "regresaron", "entraron", "salieron", "se detuvieron", "se sentaron", "continuaron", "esperaron", "asintieron", "llamaron"],
        "plural_verb": "celebraron",
    },
    "fr": {
        "female": ["Marie Martin", "Sophie Bernard", "Claire Dubois", "Julie Thomas", "Anne Robert", "Louise Richard", "Camille Petit", "Alice Durand", "Emma Leroy", "Chloé Moreau"],
        "male": ["Jean Martin", "Pierre Bernard", "Louis Dubois", "Paul Thomas", "André Robert", "Luc Richard", "Michel Petit", "Henri Durand", "Marc Leroy", "François Moreau"],
        "singular": "Elle", "plural": "Ils",
        "unique": ["est arrivée", "est revenue", "est entrée", "est partie", "s'est arrêtée", "s'est assise", "a continué", "a attendu", "a hoché la tête", "a appelé"],
        "pronoun_verb": ["a souri", "a salué", "a répondu", "est partie", "est restée", "a parlé", "a chanté", "a couru", "a attendu", "a hoché la tête"],
        "ambiguous": "{a} a rencontré {b}, et {p} {v}.",
        "amb_verb": ["a souri", "a salué", "a répondu", "est partie", "est restée", "a parlé", "a chanté", "a couru", "a attendu", "a hoché la tête"],
        "unresolved": "{p} {v}.",
        "pair": "{a} et {b} sont arrivés. {p} ont célébré.",
        "male_verb": ["sont arrivés", "sont revenus", "sont entrés", "sont partis", "se sont arrêtés", "se sont assis", "ont continué", "ont attendu", "ont hoché la tête", "ont appelé"],
        "plural_verb": "ont célébré",
    },
    "id": {
        "female": ["Siti Putri", "Dewi Lestari", "Rina Sari", "Ani Wulandari", "Maya Pratiwi", "Nia Safitri", "Ratna Dewi", "Lina Marlina", "Ayu Permata", "Fitri Handayani"],
        "male": ["Budi Santoso", "Agus Wijaya", "Rudi Hartono", "Andi Saputra", "Doni Kurniawan", "Eko Prasetyo", "Dedi Setiawan", "Joko Susilo", "Hadi Nugroho", "Ari Firmansyah"],
        "singular": "Dia", "plural": "Mereka",
        "unique": ["tiba", "kembali", "masuk", "pergi", "berhenti", "duduk", "melanjutkan perjalanan", "menunggu", "mengangguk", "menelepon"],
        "pronoun_verb": ["tersenyum", "melambaikan tangan", "menjawab", "pergi", "tetap tinggal", "berbicara", "bernyanyi", "berlari", "menunggu", "mengangguk"],
        "ambiguous": "{a} bertemu {b}, lalu {p} {v}.",
        "amb_verb": ["tersenyum", "melambaikan tangan", "menjawab", "pergi", "tetap tinggal", "berbicara", "bernyanyi", "berlari", "menunggu", "mengangguk"],
        "unresolved": "{p} {v}.",
        "pair": "{a} dan {b} tiba. {p} merayakan bersama.",
        "male_verb": ["tiba", "kembali", "masuk", "pergi", "berhenti", "duduk", "melanjutkan perjalanan", "menunggu", "mengangguk", "menelepon"],
        "plural_verb": "merayakan bersama",
    },
    "it": {
        "female": ["Anna Rossi", "Maria Bianchi", "Giulia Conti", "Sofia Romano", "Laura Gallo", "Sara Costa", "Elena Fontana", "Marta Greco", "Chiara Bruno", "Lucia Ricci"],
        "male": ["Marco Rossi", "Luca Bianchi", "Giovanni Conti", "Paolo Romano", "Matteo Gallo", "Andrea Costa", "Franco Fontana", "Stefano Greco", "Carlo Bruno", "Pietro Ricci"],
        "singular": "Lei", "plural": "Loro",
        "unique": ["è arrivata", "è tornata", "è entrata", "è partita", "si è fermata", "si è seduta", "ha proseguito", "ha aspettato", "ha annuito", "ha chiamato"],
        "pronoun_verb": ["ha sorriso", "ha salutato", "ha risposto", "se n'è andata", "è rimasta", "ha parlato", "ha cantato", "ha corso", "ha aspettato", "ha annuito"],
        "ambiguous": "{a} ha incontrato {b}, e {p} {v}.",
        "amb_verb": ["ha sorriso", "ha salutato", "ha risposto", "se n'è andata", "è rimasta", "ha parlato", "ha cantato", "ha corso", "ha aspettato", "ha annuito"],
        "unresolved": "{p} {v}.",
        "pair": "{a} e {b} sono arrivati. {p} hanno festeggiato.",
        "male_verb": ["sono arrivati", "sono tornati", "sono entrati", "sono partiti", "si sono fermati", "si sono seduti", "hanno proseguito", "hanno aspettato", "hanno annuito", "hanno chiamato"],
        "plural_verb": "hanno festeggiato",
    },
    "nl": {
        "female": ["Anna de Vries", "Emma Jansen", "Sophie Bakker", "Lisa Visser", "Julia Smit", "Eva Meijer", "Sara de Boer", "Mila Mulder", "Noor Bos", "Tess Vos"],
        "male": ["Jan de Vries", "Daan Jansen", "Pieter Bakker", "Thomas Visser", "Lucas Smit", "Bram Meijer", "Sem de Boer", "Milan Mulder", "Tim Bos", "Joost Vos"],
        "singular": "Zij", "plural": "Zij",
        "unique": ["kwam aan", "keerde terug", "ging naar binnen", "vertrok", "stond stil", "ging zitten", "ging verder", "wachtte", "knikte", "belde"],
        "pronoun_verb": ["glimlachte", "zwaaide", "antwoordde", "vertrok", "bleef", "sprak", "zong", "rende", "wachtte", "knikte"],
        "ambiguous": "{a} ontmoette {b}, en {p} {v}.",
        "amb_verb": ["glimlachte", "zwaaide", "antwoordde", "vertrok", "bleef", "sprak", "zong", "rende", "wachtte", "knikte"],
        "unresolved": "{p} {v}.",
        "pair": "{a} en {b} kwamen aan. {p} vierden feest.",
        "male_verb": ["kwamen aan", "keerden terug", "gingen naar binnen", "vertrokken", "stonden stil", "gingen zitten", "gingen verder", "wachtten", "knikten", "belden"],
        "plural_verb": "vierden feest",
    },
    "pl": {
        "female": ["Anna Kowalska", "Maria Nowak", "Zofia Wójcik", "Julia Kowalczyk", "Lena Kamińska", "Alicja Lewandowska", "Natalia Zielińska", "Oliwia Szymańska", "Maja Woźniak", "Ewa Dąbrowska"],
        "male": ["Jan Kowalski", "Piotr Nowak", "Adam Wójcik", "Paweł Kowalczyk", "Marek Kamiński", "Tomasz Lewandowski", "Jakub Zieliński", "Kacper Szymański", "Michał Woźniak", "Kamil Dąbrowski"],
        "singular": "Ona", "plural": "Oni",
        "unique": ["przyjechała", "wróciła", "weszła", "wyszła", "zatrzymała się", "usiadła", "poszła dalej", "czekała", "skinęła głową", "zadzwoniła"],
        "pronoun_verb": ["uśmiechnęła się", "pomachała", "odpowiedziała", "wyszła", "została", "rozmawiała", "śpiewała", "pobiegła", "czekała", "skinęła głową"],
        "ambiguous": "{a} spotkała {b}, a {p} {v}.",
        "amb_verb": ["uśmiechnęła się", "pomachała", "odpowiedziała", "wyszła", "została", "rozmawiała", "śpiewała", "pobiegła", "czekała", "skinęła głową"],
        "unresolved": "{p} {v}.",
        "pair": "{a} i {b} przyjechali. {p} świętowali.",
        "male_verb": ["przyjechali", "wrócili", "weszli", "wyszli", "zatrzymali się", "usiedli", "poszli dalej", "czekali", "skinęli głową", "zadzwonili"],
        "plural_verb": "świętowali",
    },
    "pt": {
        "female": ["Maria Silva", "Ana Santos", "Beatriz Costa", "Joana Pereira", "Inês Oliveira", "Sofia Rodrigues", "Mariana Ferreira", "Carolina Alves", "Clara Sousa", "Laura Gomes"],
        "male": ["João Silva", "Pedro Santos", "Paulo Costa", "Rui Pereira", "Miguel Oliveira", "Tiago Rodrigues", "Lucas Ferreira", "Carlos Alves", "Bruno Sousa", "André Gomes"],
        "singular": "Ela", "plural": "Eles",
        "unique": ["chegou", "voltou", "entrou", "saiu", "parou", "sentou-se", "continuou", "esperou", "assentiu", "telefonou"],
        "pronoun_verb": ["sorriu", "acenou", "respondeu", "saiu", "ficou", "falou", "cantou", "correu", "esperou", "assentiu"],
        "ambiguous": "{a} encontrou {b}, e {p} {v}.",
        "amb_verb": ["sorriu", "acenou", "respondeu", "saiu", "ficou", "falou", "cantou", "correu", "esperou", "assentiu"],
        "unresolved": "{p} {v}.",
        "pair": "{a} e {b} chegaram. {p} comemoraram.",
        "male_verb": ["chegaram", "voltaram", "entraram", "saíram", "pararam", "sentaram-se", "continuaram", "esperaram", "assentiram", "telefonaram"],
        "plural_verb": "comemoraram",
    },
    "ru": {
        "female": ["Анна Иванова", "Мария Смирнова", "Елена Кузнецова", "Ольга Попова", "Наталья Васильева", "Татьяна Петрова", "Ирина Соколова", "Екатерина Михайлова", "Юлия Новикова", "София Фёдорова"],
        "male": ["Иван Иванов", "Сергей Смирнов", "Алексей Кузнецов", "Дмитрий Попов", "Андрей Васильев", "Николай Петров", "Михаил Соколов", "Владимир Михайлов", "Павел Новиков", "Артём Фёдоров"],
        "singular": "Она", "plural": "Они",
        "unique": ["пришла", "вернулась", "вошла", "ушла", "остановилась", "села", "продолжила путь", "подождала", "кивнула", "позвонила"],
        "pronoun_verb": ["улыбнулась", "помахала", "ответила", "ушла", "осталась", "говорила", "пела", "побежала", "подождала", "кивнула"],
        "ambiguous": "{a} встретила {b}, и {p} {v}.",
        "amb_verb": ["улыбнулась", "помахала", "ответила", "ушла", "осталась", "говорила", "пела", "побежала", "подождала", "кивнула"],
        "unresolved": "{p} {v}.",
        "pair": "{a} и {b} пришли. {p} отпраздновали.",
        "male_verb": ["пришли", "вернулись", "вошли", "ушли", "остановились", "сели", "продолжили путь", "подождали", "кивнули", "позвонили"],
        "plural_verb": "отпраздновали",
    },
    "tr": {
        "female": ["Ayşe Yılmaz", "Fatma Demir", "Zeynep Kaya", "Elif Çelik", "Merve Şahin", "Esra Yıldız", "Derya Aydın", "Selin Özdemir", "Aylin Arslan", "Aylin Kılıç"],
        "male": ["Mehmet Yılmaz", "Mustafa Demir", "Ahmet Kaya", "Ali Çelik", "Emre Şahin", "Burak Yıldız", "Murat Aydın", "Onur Özdemir", "Cem Arslan", "Can Kılıç"],
        "singular": "O", "plural": "Onlar",
        "unique": ["geldi", "geri döndü", "içeri girdi", "ayrıldı", "durdu", "oturdu", "yoluna devam etti", "bekledi", "başını salladı", "aradı"],
        "pronoun_verb": ["gülümsedi", "el salladı", "yanıt verdi", "ayrıldı", "kaldı", "konuştu", "şarkı söyledi", "koştu", "bekledi", "başını salladı"],
        "ambiguous": "{a}, {b} ile görüştü ve {p} {v}.",
        "amb_verb": ["gülümsedi", "el salladı", "yanıt verdi", "ayrıldı", "kaldı", "konuştu", "şarkı söyledi", "koştu", "bekledi", "başını salladı"],
        "unresolved": "{p} {v}.",
        "pair": "{a} ve {b} geldi. {p} kutlama yaptı.",
        "male_verb": ["geldi", "geri döndü", "içeri girdi", "ayrıldı", "durdu", "oturdu", "yoluna devam etti", "bekledi", "başını salladı", "aradı"],
        "plural_verb": "kutlama yaptılar",
    },
}


def build() -> dict:
    languages = {}
    for language, data in LANGUAGE_DATA.items():
        cases = []
        for index in range(10):
            female = data["female"][index]
            female_competitor = data["female"][(index + 1) % 10]
            male = data["male"][index]
            male_competitor = data["male"][(index + 1) % 10]
            pronoun = data["singular"]
            plural = data["plural"]
            cases.extend((
                {
                    "case_id": f"{language}_resolved_{index + 1:02}",
                    "text": f"{female} {data['unique'][index]}. {pronoun} {data['pronoun_verb'][index]}.",
                    "reference": pronoun, "expected_status": "resolved",
                    "expected_targets": [female], "scenario": "resolved_singular",
                },
                {
                    "case_id": f"{language}_ambiguous_{index + 1:02}",
                    "text": data["ambiguous"].format(
                        a=female, b=female_competitor, p=pronoun.lower(), v=data["amb_verb"][index]
                    ),
                    "reference": pronoun, "expected_status": "ambiguous",
                    "expected_targets": [female, female_competitor], "scenario": "same_gender_competitors",
                },
                {
                    "case_id": f"{language}_unresolved_{index + 1:02}",
                    "text": data["unresolved"].format(p=pronoun, v=data["pronoun_verb"][index]),
                    "reference": pronoun, "expected_status": "unresolved",
                    "expected_targets": [], "scenario": "no_antecedent",
                },
                {
                    "case_id": f"{language}_plural_{index + 1:02}",
                    "text": data["pair"].format(
                        a=male, b=male_competitor, p=plural, v=data["plural_verb"]
                    ),
                    "reference": plural, "expected_status": "resolved",
                    "expected_targets": [male, male_competitor], "scenario": "plural_pair",
                },
            ))
        languages[language] = cases
    return {
        "schema": "brainrust.coref_gold.v1",
        "annotation_authority": "assistant_draft_pending_human_review",
        "case_design": "40 draft pronoun references per language: 10 each resolved singular, same-gender competitors, no antecedent, and plural pair.",
        "languages": languages,
    }


if __name__ == "__main__":
    OUTPUT.write_text(json.dumps(build(), ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {OUTPUT}")
