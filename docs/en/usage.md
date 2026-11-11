**English** | [日本語](../ja/usage.md) | [简体中文](../zh-CN/usage.md)

# Using Xexam

> How to use Xexam in the browser: exams and practice, study records, settings, the question banks and their Excel format, users and the System log. Installing, updating and configuring Xexam are in the [README](../../README.md) and in [deployment.md](deployment.md).
>
> The screenshots use two **made-up demo banks**, "General knowledge" and "Kitchen basics", not content from any real exam. Both are six-column workbooks, so every question, answer and explanation exists in English, Chinese and Japanese. The screenshots show the English interface; those of the Japanese and the Chinese interface are in `docs/ja/screenshots/` and `docs/zh-CN/screenshots/`.

## Contents

- [1. Your first exam](#1-your-first-exam)
- [2. The layout](#2-the-layout)
- [3. Home](#3-home)
- [4. The exam page](#4-the-exam-page)
- [5. Practice](#5-practice)
- [6. The result page](#6-the-result-page)
- [7. Study records](#7-study-records)
- [8. Wrong-answer notebook](#8-wrong-answer-notebook)
- [9. Settings](#9-settings)
- [10. About Xexam](#10-about-xexam)
- [11. Question banks](#11-question-banks)
- [12. Excel question bank format](#12-excel-question-bank-format)
- [13. Importing question banks](#13-importing-question-banks)
- [14. Users and sign-in](#14-users-and-sign-in)
- [15. System log](#15-system-log)
- [16. On an iPhone](#16-on-an-iphone)
- [17. When a page says "Too many requests"](#17-when-a-page-says-too-many-requests)

## 1. Your first exam

1. Open the menu (the three-line button at the top right) and choose **Question banks** (administrators and bank managers see it). Under **Import Excel**, leave **New bank** selected, click **Choose Excel file** and pick your workbook (in the six-column layout, see [section 12](#12-excel-question-bank-format)). Check the preview, then click **Create the question bank**.
2. Click **Start an exam** in the message that confirms the import (or choose **Home** in the menu). In the **Start an exam or practice** panel every sheet is ticked at first: untick the ones you do not want, then click **Start exam** in the bar at the bottom of the panel (**Count** and **Shuffle** in the same bar set how many questions are asked and in which order). **Start practice** next to it starts a [practice](#5-practice) with the same questions instead.
3. Answer the questions, then click **Submit**. The result page shows the score and, for each wrong question, the correct answer and the explanation (**All questions (n)** shows every question).
4. To see the exam again later: open the menu, choose **Study records** and click the time of the exam.

A new bank is visible to you and to the administrators, and to every user who has **Include banks imported later**. An administrator gives it to other users in [User management](#14-users-and-sign-in).

![The exam page: a hotspot question in English, the English / 中文 / 日本語 switch and the answer sheet with Submit](screenshots/desktop-2-quiz.png)

## 2. The layout

- Every page except the exam has a slim header: the Xexam logo at the top left (it takes you to Home); at the top right your username (administrators with an **Administrator** tag, **Admin** on a phone) and the three-line menu button.
- The menu shows only the pages of your role:
  - **Home**, **Study records** and **Settings** for everyone;
  - **Question banks** for administrators and bank managers;
  - **User management** and **System log** for administrators.
- Under a divider follow the version entry (for example **Xexam v0.7.5**), which opens the [About page](#10-about-xexam), then your username with your role (**Administrator** or **User**, plus **Bank manager**) and **Sign out**.
- The menu closes after a choice, a click outside it or the Escape key.
- On wide screens the page content is centered and at most 1120 px wide.
- The exam and the practice are focused full-screen pages without the header.
- Opening **Question banks** or **System log** without the permission shows **Not allowed** with **Back to Home**; **User management** sends everyone but administrators to Home.
- Everything that deletes asks first, in a dialog that says what goes and that it cannot be undone; its confirm button is solid red. **Delete** buttons are red, or turn red when you point at them.

## 3. Home

Before there is anything to practise, the **Start an exam or practice** panel shows a short note instead:

- an administrator, or a bank manager without any bank, gets **Go to Question banks**;
- a user who has not been given a question bank yet reads: “No question bank has been made available to you yet. Ask an administrator to give you access to one; it appears here as soon as they do.”
- when the banks have no active questions (every question has been retired, for example because it came from an older Excel layout), an administrator is asked to update them with a six-column file; a bank manager reads that they can update their own banks with a six-column Excel file or import a new one, and ask an administrator to update the others, with **Go to Question banks**; any other user is asked to turn to an administrator.

After that, Home has:

- **Unfinished exam** or **Unfinished practice**: a compact banner at the top for what you left with **Save and exit**: its title with **Saved in this browser only**, then its question banks, how many questions are answered (in a practice: checked) and the time used. **Continue** picks it up where you stopped; **Discard** throws it away. Your other devices do not see it. Starting something new while one is saved asks first.
- **Start an exam or practice**:
  - In the top-right corner of the panel you choose where the questions come from: **All** (every question in the selected sheets) or **Wrong answers** (only the questions you still need to review). Each shows its number of questions. When the notebook has nothing to review in the selected sheets, **Wrong answers** cannot be chosen and one line under the title says why; while **Wrong answers** is chosen, that line says how a question leaves the notebook. On a phone the two buttons stay next to the title; in English and Japanese they show only their icon and number, in Chinese they keep their short words.
  - **Banks and sheets**: each bank is one card with its sheets as rows inside.
    - **Select all** and **Clear** are on the right of the heading row.
    - The bank's checkbox selects or clears all its sheets, and you can also tick single sheets. While only some are selected, the checkbox shows − and the card says, for example, **1 of 2 sheets selected** (also while the bank is folded).
    - The arrow button on the right of a bank's row folds or unfolds its sheets. With at most two banks they start unfolded, with more they start folded; your choice is kept until you reload the page.
    - A badge such as **2 to review** shows how many notebook questions a bank or sheet has.
  - The bar at the bottom of the panel: **Start exam** (green) and **Start practice** (orange), the scope (for example **12 questions in scope**, or **11 of 12 in scope, picked at random**) and two options, **Count** (the number of questions) and **Shuffle** (the question order). Both buttons use the same scope. The bar stays at the bottom of the screen while the panel is in view.
  - **Count**: when it is smaller than the scope, the questions are picked at random from the whole scope. **Shuffle** only decides whether they are asked in random order or in bank order.
- **Latest result**: the score of your latest exam or practice with its **Exam** / **Practice** tag, **View details** and **All study records**.
- **Overview**: how many question banks, usable sheets, active questions and notebook questions you have, with **Review wrong answers** (when the notebook has questions) and, for administrators and bank managers, **Question banks**.

![Home page on a desktop](screenshots/desktop-1-home.png)

The unfinished exam as a compact banner; **All** and **Wrong answers** in the corner of **Start an exam or practice**; **Select all** and **Clear** on the row of **Banks and sheets**; the two demo banks as unfolded cards with their sheets; the bar with the green **Start exam**, the orange **Start practice**, the scope, **Count** and **Shuffle**; on the right, the latest result and the overview.

## 4. The exam page

The screenshot in [section 1](#1-your-first-exam) shows it.

- At the top: **Question 4 of 12** (for example), the time used, and **Save and exit**, which keeps your answers and the time used so you can continue later from Home in the same browser.
- **No right/wrong feedback and no score while you answer.**
- **English** / **中文** / **日本語** switches the question text. Switching changes only the text: your answers, the option letters, the timer and the grading stay the same. The option buttons of hotspot questions switch too.
  - English is the base language of every question: it is always there. Chinese and Japanese are there when the workbook has them.
  - A language that a question does not have, or whose text does not line up with the English one (other options or items), is greyed out for that question; tapping it says why.
  - Your choice is saved as a setting and is also where the answers and explanations start after you submit.
- **Previous** and **Next** move between questions (on the last question, **Next** becomes **Submit**). **Mark to revisit** marks a question you want to come back to.
- **Answer sheet**: one tile per question. Solid = answered, half-filled = a hotspot question partly answered, hollow = not answered, small yellow square = marked to revisit. Click a tile to jump there. Below the tiles: **Answered 3 of 12** and **Submit**.
- **Submit** asks for confirmation; unanswered questions count as wrong.
- Grading: a single-choice or multiple-choice question is correct only when you chose exactly the correct options; a hotspot question is correct only when every item is right.
- Keyboard (desktop): `←` `→` previous / next question, `A`–`H` choose an answer (choice questions), `M` mark to revisit.

## 5. Practice

A practice uses the same page as the exam, but checks every answer right away. Start it with **Start practice** on Home, next to **Start exam** and with the same banks, sheets, **Count** and **Shuffle**.

- A **Practice** tag sits next to **Question 4 of 12**.
- **How an answer is checked**:
  - a single-choice question is checked as soon as you pick an option;
  - a multiple-choice or hotspot question has **Check answer**, which works once something is chosen. For a hotspot question one item is enough; items left open count as wrong.
- **After checking**, the question is locked and shows:
  - **Correct** or **Wrong**;
  - the options marked as in the answers and explanations: the correct ones solid green, a wrongly chosen one solid orange;
  - your answer and the correct answer;
  - the explanation and the memory aid, in the language of the page's **English** / **中文** / **日本語** switch.
- Auto-advance never applies in a practice: you move on with **Next**.
- The answer sheet fills checked questions green (correct) or orange (wrong), and says, for example, **Checked 4 of 12 · 3 correct**.
- **Finish practice** (below the answer sheet, and on the last question) asks only when some questions are not checked yet. Those count as not answered (wrong), even when an answer is chosen.
- **Save and exit** works as in an exam; Home then shows **Unfinished practice** with how many questions are checked.
- A finished practice opens the **Practice result** page, is saved in **Study records** as a **Practice** record, and counts in the wrong-answer notebook like an exam.

![The practice page after Check answer on a multiple-choice question answered wrongly: the Practice tag, the verdict, the chosen wrong option solid orange, the correct options solid green, the explanation and the memory aid](screenshots/desktop-11-practice.png)

A multiple-choice question after **Check answer**: the **Practice** tag, the verdict **Wrong**, the wrongly chosen option in solid orange, the correct options in solid green, then the explanation and the memory aid.

## 6. The result page

The score, the number right and wrong, and a strip with one box per question (a green tick for a correct one). A practice shows **Practice result** as its title.

**Answers and explanations** shows **Wrong only (n)** or **All questions (n)**. Each question card shows:

- **Correct** or **Wrong**, and your choice and the correct answer: correct options solid green, a wrongly chosen option solid orange, each with its **Correct answer** / **Your choice** tag (hotspot statements sit on a neutral background; only their choices are coloured);
- the line **Your answer** / **Correct answer**: in a multiple-choice answer each letter of yours is green when it is one of the correct answers and orange when it is not, so a partly right answer shows both colours;
- its own **English** / **中文** / **日本語** switch. It starts at your question language and changes only that card: the question, the options, the explanation, the memory aid and the note. Languages the question does not have are greyed out;
- the explanation and the memory aid, in each language the workbook has them; if one language lacks them, another language's text is shown;
- how often you have answered the question so far.

If questions were deleted (with their bank) while you took the exam, a note says so: the score and the saved record cover only the other questions.

Buttons: **Retry wrong answers only**, **Take it again** (**Retake practice** after a practice), **View in study records** and **Back to home**. Retrying and retaking start the same kind as the result: an exam again as an exam, a practice as a practice.

![Result page on a desktop](screenshots/desktop-3-result.png)

An exam with two wrong answers, **Wrong only** open: the score, then a choice question with the chosen option in solid orange and the correct ones in solid green, its own **English** / **中文** / **日本語** switch, and the explanation and the memory aid.

## 7. Study records

- **Overview**: the number of records (**Records**, or **Exams** / **Practice sessions** when the type filter says so), **Average accuracy**, **Best accuracy** and small bars for the latest results.
- **Filters**: **Type** (**All types**, **Exams**, **Practice**), **Question bank**, a date range with **From** and **To**, and **Accuracy** (**Any accuracy**, **Below 60%**, **60–79%**, **80% and above**). **Clear filters** resets them.
- **Records**: one row per exam or practice (on a phone: one card) with its **Exam** / **Practice** tag, the time, the question banks, how many questions were answered, the time used, the number correct and the accuracy (green from 80 %, orange below 60 %). Click the time to open the record; **Delete** removes it.
- **Exam record** / **Practice record** (one record): the score and the facts (with **Type**), then every question with your answer, the correct answer, the explanation and the memory aid, each card with its own language switch. **Wrong only (n)** / **All questions (n)** switches between the wrong questions and all of them. **Retry wrong answers** starts the questions you got wrong; **Retake this exam** (**Retake this practice**) starts the same questions again, both as the same kind as the record; **Delete this record** removes it. Exams taken before v0.5 show only the score, because v0.4 did not store the answers. A retired question still shows in a record with its text and a **Retired** tag; only questions that are active now are retried or retaken.
- **Review wrong answers** (top right) opens **Wrong-answer review**: every active question in your wrong-answer notebook, with how often you answered it and how many more correct answers in a row take it out. You can narrow it to one question bank, and **Practise these** starts a practice with them.
- Deleting: deleting one record keeps the per-question statistics, so the wrong-answer notebook does not change. **Delete all** deletes every record (exams and practice); tick **Also clear the wrong-answer notebook and per-question stats** to clear those too. Every delete asks for confirmation and cannot be undone.
- Xexam keeps your latest 500 records (exams and practice); older records are removed automatically.

![Study records on a desktop: overview, filters with the type filter, and the list of exam and practice records](screenshots/desktop-4-records.png)

## 8. Wrong-answer notebook

Questions you answer wrong, in an exam or in a practice, go into your wrong-answer notebook and leave it after you answer them correctly **2 times in a row** (in either). Use it from Home (**Wrong answers** in the top-right corner of **Start an exam or practice**), from **Study records** → **Review wrong answers** → **Practise these**, or right after an exam with **Retry wrong answers only**. A retired question stays in your statistics but is not offered for review; it comes back if a later import of its bank restores it.

## 9. Settings

- **Account**: your **Username** and **Role** (with **Bank manager** if you have that permission), and **Change password**. A bank manager also reads what the permission allows.
- **Appearance** (the theme): **System** (the default; follows the light or dark setting of your device), **Light** or **Dark**.
- **Language** (the interface language): **English** (the default), **中文** or **日本語**. It changes the menus, buttons and messages everywhere, the exam page and the error messages from the server included. The question text has its own switch on the exam page.
- **Answering** → **Auto-advance to the next question** (off by default, exams only): after you answer a single-choice question, or every item of a hotspot question, the next question opens by itself. Multiple-choice questions never move on by themselves. While it is on, the exam page shows **Auto-advance on** at the top.
- **Reset to defaults**: theme, interface language and auto-advance go back to their defaults. The question language is kept.

Settings belong to your account and are saved on the Xexam server, together with the question language you last chose. So they survive restarts and image updates, clearing the browser cache, and switching to another browser or device; every user has their own. Only an unfinished exam or practice stays in the current browser.

<table>
<tr>
<td width="50%"><img src="screenshots/desktop-6-settings.png" alt="Settings page on a desktop"></td>
<td width="50%"><img src="screenshots/desktop-7-dark.png" alt="Exam page in the dark theme"></td>
</tr>
<tr>
<td>Settings: <b>Account</b>, <b>Appearance</b>, <b>Language</b> and <b>Answering</b>.</td>
<td>The <b>Dark</b> theme, here on the exam page.</td>
</tr>
</table>

## 10. About Xexam

The version entry of the menu (for example **Xexam v0.7.5**) opens **About Xexam**: the **Version**, the CPU architecture of the **Server**, the **License**, and **What’s new**, which is the changelog of the README in the interface language. The newest version is open; the earlier ones are folded under **Earlier versions**.

![About Xexam on a desktop, with What’s new showing the newest version and the earlier versions folded](screenshots/desktop-8-about.png)

## 11. Question banks

Here administrators and bank managers import Excel workbooks. Users without the permission do not see the page in the menu; opening it directly shows **Not allowed** with **Back to Home**.

- **Import Excel**: pick **New bank** or **Update a bank** (two small buttons next to the title; their tooltips explain them; for an update you choose the bank under **Bank to update**), then click **Choose Excel file** or drop the file anywhere on the panel. The hint line says to upload the whole workbook in the six-column layout and ends in a link to the **Excel format guide**.
- **The bank list**, one compact row per bank: its file, number of questions (with the retired ones, for example **5 questions (+2 retired)**) and sheets and last update, with **Update** and **Delete**. Its sheets are folded under the arrow on the right of the row. A bank whose questions are all retired because they came from an older Excel layout says so, and that an update with a six-column file brings it back into use.
  - An **administrator** sees **All question banks**, each with who imported it (**uploaded by sam**, **uploaded by you**), and may update or delete any of them.
  - A **bank manager** sees **Your question banks**: only the banks they imported, which they may update or delete. Banks an administrator gave them to study are on Home.
- **Import history**: one line per import, the latest 8 (an administrator sees all imports, a bank manager those of their own banks).
- **Excel format and import guide**, folded until you open it, is the short version of [section 12](#12-excel-question-bank-format) and [section 13](#13-importing-question-banks).

![Question banks page as an administrator: the preview of an update of “Kitchen basics” with a warning about a differing Japanese answer, the bank list with who uploaded each bank, and the Excel format guide opened at its first sections](screenshots/desktop-5-banks.png)

The screenshot shows the preview of an update of "Kitchen basics" with a newer file, `Kitchen basics (v2).xlsx`, before anything is saved: the chips (1 added, 1 updated, 1 retired), the question that would be retired, and the warning that the Japanese answer of one row differs from the English one (the row is imported with the English answer). It also shows the bank list under **All question banks** ("Kitchen basics" was uploaded by the bank manager sam) and the **Excel format and import guide**, opened, with its first two parts: the basics and the six columns.

## 12. Excel question bank format

One workbook (`.xlsx`) is one question bank. Each sheet is a group of questions, one question per row. **Only the six-column layout can be imported**: every question has its own column in English, Chinese and Japanese, and so has its answer and explanation. A file in any other layout, the three- and four-column layouts of earlier versions included, is refused with a message that says so; nothing is imported from it.

| Column | Contents |
|---|---|
| **A** | The **English** question, then its options or hotspot items. The question type, the options and the items are taken from this column, and the question is recognised by it |
| **B** | The **Chinese** question (optional), with the same options (A, B, C…) or the same numbered items as column A |
| **C** | The **Japanese** question (optional), with the same options or items as column A |
| **D** | The **English** answer, explanation and memory aid: `【Answer】B`, then `【Explanation】…` and `【Memory aid】…`. The answer is taken from this column |
| **E** | The **Chinese** answer, explanation and memory aid (optional): `【答案】B`, `【解析】…`, `【记忆口诀】…` |
| **F** | The **Japanese** answer, explanation and memory aid (optional): `【正解】B`, `【解説】…`, `【覚え方】…` |

Here is one complete single-choice question (made up, like every example here; it is the same as in the guide on the **Question banks** page):

| Column | Cell |
|---|---|
| A | Which planet is known as the Red Planet?<br>A. Venus<br>B. Mars<br>C. Jupiter<br>D. Saturn |
| B | 哪颗行星被称为“红色星球”？<br>A. 金星<br>B. 火星<br>C. 木星<br>D. 土星 |
| C | 「赤い惑星」と呼ばれる惑星はどれですか。<br>A. 金星<br>B. 火星<br>C. 木星<br>D. 土星 |
| D | 【Answer】B<br>【Explanation】Iron oxide (rust) in the dust on its surface makes Mars look red.<br>【Memory aid】Mars rusts red. |
| E | 【答案】B<br>【解析】火星表面的尘土含有氧化铁（铁锈），所以看起来是红色的。<br>【记忆口诀】火星生锈红彤彤。 |
| F | 【正解】B<br>【解説】火星の表面の砂には酸化鉄（さび）が含まれているため、赤く見えます。<br>【覚え方】火星はさびて赤い。 |

To start a new line inside a cell, press Alt+Enter (Windows) or Control+Option+Return (Mac).

### 12.1 Rows and headings

- Each row is one question. The first row may hold headings, for example `Question (English)` … `Answer (Japanese)`: a row whose column A is a single line and whose column D has no answer heading such as `【Answer】` is treated as a heading or a note and skipped.
- Xexam recognises the six-column layout by the content of each worksheet: at least one row needs a Japanese question with kana in column C (and no kana in columns A and B) and an answer with its heading, in any of the three languages, in column E or F.

### 12.2 English is authoritative

- The question type, the options, the hotspot items and the question's identity come from column A, the answer key from column D. A row without an English question in A or an English answer in D cannot be read; the preview lists it with the reason.
- Columns B, C, E and F may be left empty in a row: without column B or C, Chinese or Japanese is greyed out on that question; without column E or F, the card shows the explanation of another language.
- Columns B and C must line up with column A (the same option letters, the same number of items). A language that does not line up is greyed out for that question.
- When the answer in column E or F means something else than column D, the row is still imported with the English answer, and the preview warns, for example **1 answer differs from the English answer**, with the rows under **Show the rows**. Answers are compared by meaning: the same letters, or the same choice for each hotspot item (`Yes`, `是` and `はい` count as the same).

### 12.3 Question types

- **Single choice**: in column A, the question first, then one option per line: `A. text`, `B. text` … (two to eight options, A to H; `A)`, `A、` and `A:` work too). The answer is one letter: `【Answer】B`.
- **Multiple choice**: written the same way, with several letters in the answer: `【Answer】C, E` or `【Answer】CE`. An answer counts as right only when exactly these options are chosen.
- **Hotspot** (several statements, one choice for each): the first line of column A is `HOTSPOT` (recommended: it marks the question as a hotspot one; without it, numbered items that end in choices are recognised too), then the question, then one numbered item per line that ends with its choices, separated by ` / `: `1. Water is wet. Yes / No`. Choices of more than one word (such as `Before cooking / After cooking`) need a tab or a full-width space (`　`) in front of them. The answer gives one choice per item, separated by `;`: `【Answer】1 Yes; 2 No`.
- In columns B and C, write the same items with the same numbers; when they end with their own choices (`是 / 否`, `はい / いいえ`), the question shows those in that language. In every answer column, yes and no may be written in any of the three languages: `Yes` / `是` / `はい` and `No` / `否` / `いいえ`.

For example, this hotspot question (columns A and D; columns B, C, E and F hold the same in Chinese and Japanese, with a full-width space before `是 / 否` and `はい / いいえ`):

```
HOTSPOT
For each statement, choose Yes or No.
1. The sun rises in the east. Yes / No
2. The moon is a star. Yes / No
```

```
【Answer】1 Yes; 2 No
【Explanation】The sun rises in the east. The moon only reflects sunlight; it is not a star.
```

### 12.4 Headings in the answer columns

In columns D, E and F, write the answer first, then the explanation and the memory aid, each after its heading in `【…】`. Each of the three columns accepts the headings of all three languages:

- Answer: `【Answer】`, `【答案】`, `【正解】` or `【答え】`.
- Explanation: `【Explanation】`, `【解析】` or `【解説】`.
- Memory aid: `【Memory aid】`, `【Mnemonic】`, `【记忆口诀】`, `【口诀】` or `【覚え方】`.

English headings may be written in any case, and a longer heading that contains one of these words, such as `【解析·怎么选】`, counts too. A cell that holds nothing but the answer may also be just the letters (`B`) or one line such as `Answer: B`. Text in parentheses after the answer, for example `【Answer】B (see chapter 2)`, is not part of the answer: it is shown as a **Note** on the answer cards.

### 12.5 Questions that cannot be read

Questions that cannot be read are **never guessed**: they are listed in the preview with the reason (usually column D does not contain an answer with its heading, or the answer letter is not among the options), and they are never silently marked wrong.

## 13. Importing question banks

### 13.1 Who may import

- **Administrators** import new banks and update or delete every bank.
- **Bank managers** (users with the **Bank manager** permission, set in [User management](#14-users-and-sign-in)) import new banks, which then belong to them, and update or delete only the banks they imported. Banks of administrators and of other users stay out of their reach.
- Other users do not see the **Question banks** page.

### 13.2 New bank or update

A question bank is one imported Excel workbook. On **Question banks** → **Import Excel** you choose where the file goes with two small buttons next to the title:

| Choice | What happens |
|---|---|
| **New bank** | A new bank is created, named after the file (without `.xlsx`; ` (2)`, ` (3)`, … is added if the name is taken). Existing banks are not changed. The same workbook can be imported twice as two separate banks; their study records do not mix |
| **Update a bank** | The file replaces the questions of the bank you pick under **Bank to update** (a bank manager can pick only their own banks). Questions missing from the file are retired **in that bank only**. The bank keeps its name, its owner and the users who see it |

Use **New bank** for a different workbook, and **Update a bank** for a newer version of a workbook you have already imported. Always upload the **whole** workbook, not only the new questions.

A new bank is visible to the person who imported it, to the administrators, and to every user with **Include banks imported later**. An administrator gives it to other users in **User management**.

### 13.3 Preview, then confirm

After the upload Xexam reads and checks the file and shows a preview: small chips with how many questions will be **Added**, **Updated**, **Restored** or **Retired** and how many are **Unchanged** and **Unreadable** (counts of zero are hidden), one line per sheet with its numbers, and for every row that could not be read, the row number and the reason, folded under the line of its sheet (for example **2 unreadable**). **Nothing is written until you confirm** (**Create the question bank** or **Confirm import**; **Cancel, do not import** drops it). The preview also warns you when:

- you create a new bank from a file with the same name as the file an existing bank was imported from (it offers to update that bank instead);
- you update a bank with a file whose name differs from the last import;
- questions would be retired. It lists some of them, and asks once more before the import;
- the Chinese or Japanese answer of a row differs from the English one (the row is imported with the English answer).

Every person who imports has their own preview, kept in memory for 30 minutes. All open previews share a memory budget: when several large files are being previewed at the same time, the oldest preview of another person can be dropped. Confirming a preview that was dropped or is older than 30 minutes says that it has expired: upload the file again.

### 13.4 Import rules

The file is treated as the **complete content of the bank** it goes to. Other banks are never touched.

| Case | Result |
|---|---|
| A question is in the file but not in the bank | **Added** |
| Same question, but other content (other options, answer, explanation, Chinese or Japanese text) changed | **Updated** (answer history is not affected) |
| A question is in the bank but cannot be found in the file | **Retired**: it stays saved with its answer history, but no longer appears in exams or practice |
| A question that was retired earlier appears again | **Restored**, and its wrong-answer notebook entries come back with it |
| The sheet name, the English question or the first option changed | Counts as **a new question**: the old one is retired, and the new one starts without answer history |
| The file contains not a single usable question | Import rejected (this prevents wiping the bank) |

A question is recognised by its worksheet name, its English question (column A without the options) and its first option or item (spaces do not matter).

### 13.5 Banks from an older Excel layout

Up to v0.6 Xexam imported workbooks in a three-column and a four-column layout, whose questions were recognised by their Japanese text. Since v0.7 these layouts can no longer be imported, and **every question that was imported from them is retired at every start** of the backend. The banks, their import history, the study records and the per-question statistics stay; the questions are no longer asked, and a bank whose questions are all retired says so in the bank list.

To use such a bank again, **update it with a six-column version of its workbook** (**Update a bank**). The bank keeps its name, its owner and the users who see it. Because the six-column layout recognises questions by their English text, every question of the file is added as a new question, without answer history; the old ones stay retired, and the old records still show them. You can also import the six-column file as a **New bank** and delete the old one.

### 13.6 Deleting a bank

**Delete** on a bank’s row asks for confirmation (for example **Delete “Kitchen basics”?**). The bank's questions (active and retired) and its import history are deleted, and it disappears from every user's question banks. The checkbox below is off by default:

- **Unchecked**: the answer history and the exam records stay. In those records, the bank's questions show as deleted.
- **Checked**, as an administrator (**Also delete this bank’s study records**): the answer history of the bank's questions and the records that no other remaining bank is part of are deleted too.
- **Checked**, as a bank manager (**Also delete your study records of this bank**): only your own answer history and records are deleted; other users' study records always stay, with the questions shown as deleted.

A delete cannot be undone; your own backup is the only way back (section 5 of [database.md](database.md)).

## 14. Users and sign-in

Since v0.6 everyone signs in. There are three kinds of account:

- **Administrator**: everything, including every question bank, **User management** and the **System log**.
- **User** with the **Bank manager** permission: imports new question banks and updates or deletes the banks they imported, but not anybody else's.
- **User**: takes exams and practises with the question banks an administrator gave them.

Each user has their own study records, wrong-answer notebook, unfinished exam and settings.

![User management: administrators, a bank manager who sees every question bank, including ones imported later, a user with one question bank and a disabled user](screenshots/desktop-10-users.png)

User management with two administrators (admin and lee), sam (**Bank manager**, every question bank, including ones imported later), alex (one question bank, General knowledge) and kim (disabled).

- **First start** (a new install, or the first start after upgrading from v0.5 or older): sign in with `admin` / `admin` (the sign-in page reminds you of it). Xexam then asks for a new password (8–128 characters). The study records, notebook and settings from before v0.6 belong to this administrator.
- **User management** (menu, administrators only): one row per user with role, status (**Normal**, **Disabled**, **Locked · … left**, **3 tries left**, **Must choose a new password**), **Bank manager** when the user has that permission, created, last sign-in and the number of study records (exams and practice). Actions: **Edit**, **Reset password**, **Disable** / **Enable**, **Delete** and, when needed, **Unlock**. A user created or reset by an administrator chooses their own password at the next sign-in. You cannot disable, demote or delete yourself, and the last enabled administrator cannot be removed. The list refreshes itself while it is open, so a lock that starts shows up.
- **Which question banks a user sees** is two independent settings: the existing banks ticked for them, and **Include banks imported later**, which gives them every bank imported from then on (by anyone, also with the command-line `import`). It never adds an existing bank.
  - Each user's row says what they have: **Question banks: all, including ones imported later** (every bank ticked and the later ones included), **Question banks: 1 · General knowledge** (with **, and the ones imported later** when that is on), **Question banks: none yet, so this user sees no question bank**, or **Question banks: none for now; the ones imported later are added automatically**. A bank manager's own uploads are listed below it.
  - **Change** opens **Question banks of alex** (for example): **Include banks imported later** and **Select all** on the first line, then one box per bank with who uploaded it. **Select all** ticks every listed bank, or unticks them all when all are ticked; it does not touch **Include banks imported later**.
  - The change applies at once and the user stays signed in. Administrators always see every bank. Taking a bank away keeps the user's study records as history. A bank manager always sees the banks they imported, while they have the permission.
- **Add user**: username, initial password, role, the question bank checklist (nothing ticked and **Include banks imported later** off: a new user sees no question bank until you choose some) and **Bank manager**. A new account is enabled. The last two are greyed out for an administrator, who sees and manages everything anyway. **Edit** changes the username, the role, **Bank manager** and **Account enabled**. Taking the **Bank manager** permission away applies at the user's next page change.
- **Deleting a user** also deletes their study records (the dialog says how many), wrong-answer notebook and settings. Question banks they imported are not deleted: they move to you, the administrator who deletes the user.
- **Wrong passwords**: 5 wrong passwords for an account within 15 minutes lock its sign-in for 1 minute; after that, every 3 more lock it again, for 5 minutes, 15 minutes, 30 minutes, 1 hour and then 1 day. The lock belongs to the account, not to the typed name, and guessing other usernames cannot wear it away. Browsers that signed in to the account before are counted separately, so guesses from elsewhere do not lock them out. A successful sign-in, **Unlock**, a password reset by an administrator, a restart of the backend, or 24 hours without a new mistake start over with 5 tries. The sign-in page says how long to wait.
  - In **User management** a short note under the row says which browsers are locked, until when, and how many locks in a row there were. **Unlock** gives 5 tries again at once, without asking; it is also offered while the user still has tries left.
- **Change your password**: **Settings** → **Account** → **Change password**; your other signed-in browsers are signed out.
- **Forgot the administrator password**: `docker compose exec backend xexam-backend reset-password admin` prints a new temporary password, which must be replaced at the next sign-in. It works while Xexam runs. To clear a lock at once, restart the backend: `docker compose restart backend`.

<table>
<tr>
<td width="50%"><img src="screenshots/desktop-9-signin.png" alt="The sign-in page"></td>
<td width="50%"><img src="screenshots/desktop-12-user-banks.png" alt="The dialog Question banks of alex: Include banks imported later and Select all on the first line, then the banks with who uploaded each, General knowledge ticked"></td>
</tr>
<tr>
<td>The sign-in page.</td>
<td><b>Question banks of alex</b>: <b>Include banks imported later</b> and <b>Select all</b>, then the banks with who uploaded each; here only General knowledge is ticked.</td>
</tr>
</table>

## 15. System log

**System log** (menu, administrators only; everybody else gets **Not allowed**) shows what the backend has logged since it last started, newest at the bottom: startup, sign-ins, changes to users, rejected requests or uploads and so on.

- The page reads new lines every 0.5 seconds while it is open and follows them as they arrive. Scroll up to read, and **Jump to latest** takes you back down. When reading fails, it waits longer before the next try (up to 10 seconds) and shows why; a read that takes more than 10 seconds is cut off and counts as failed.
- **Pause** stops reading new lines for a while; **Resume** goes on.
- **Level** shows **All** lines, only **Warnings** (and errors) or only **Errors**; **Search the log** filters by text.
- Only the last 2000 lines are kept, in memory only: the list is empty again after a restart. For older lines, run `docker compose logs backend` on the server, in the folder of the compose file (without Compose: `docker logs xexam-backend`).
- The lines name users by their username (`user "admin" updated user "alex"`; a rename adds `(renamed from "…")`). A name typed at sign-in that has no account is never written down. The lines never contain passwords, session tokens, cookie values or question text. Rejected requests and failed sign-ins that come in bursts are summed up once a minute, so nobody can push the other lines out without signing in.

![The System log page: startup lines, sign-ins named by username, users created and updated by an administrator, and the lock of a made-up username, with the level filter, the search and Pause](screenshots/desktop-13-logs.png)

The System log with lines from the startup, sign-ins of admin, sam and alex, alex changing their password, the administrator creating lee and updating kim, and a made-up username that is guessed until it locks; above them the **Level** filter, the search field and **Pause**, below them the note on where older lines are.

## 16. On an iPhone

- Open the service address in Safari on your iPhone (first follow section 4 of [deployment.md](deployment.md) so the phone can reach Xexam). It needs **iOS 15.4 or later**.
- Tap the Share button → **Add to Home Screen**. After that it opens full-screen like a standalone app, with an Xexam icon in the style of an answer sheet.
- The three-line button at the top right opens the same small menu as on a computer.
- On Home, **All** and **Wrong answers** stay next to the **Start an exam or practice** title (in English and Japanese with only their icon and number). The bar at the bottom of the screen has two rows: the scope, **Count** and **Shuffle** on the first; **Start exam** (the wider one) and **Start practice** on the second.
- On the exam and practice page, **Previous**, **Next** and **Mark to revisit** stay at the bottom, and the answer sheet sits below the question with **Answered x of n** and **Submit** (or **Finish practice**) above the tiles.
- Study records are shown as compact cards.
- Buttons and other touch targets are at least 44 px; the safe areas for the notch and the bottom bar are handled.

Screenshots on an iPhone (390 px wide):

<table>
<tr>
<td width="25%"><img src="screenshots/iphone-1-home.png" alt="Home page on an iPhone"></td>
<td width="25%"><img src="screenshots/iphone-2-quiz.png" alt="Exam page on an iPhone"></td>
<td width="25%"><img src="screenshots/iphone-3-menu.png" alt="The menu of an administrator open on an iPhone"></td>
<td width="25%"><img src="screenshots/iphone-4-records.png" alt="Study records on an iPhone"></td>
</tr>
<tr>
<td>Home: the scope, <b>Count</b> and <b>Shuffle</b>, with <b>Start exam</b> and <b>Start practice</b> below them, at the bottom of the screen.</td>
<td>Exam: <b>Previous</b>, <b>Next</b> and <b>Mark to revisit</b> stay at the bottom.</td>
<td>The menu of an administrator, opened from the three-line button.</td>
<td>Study records: each record is a compact card.</td>
</tr>
</table>

## 17. When a page says "Too many requests"

Xexam's Nginx limits how many requests reach the backend per second from one address (section 9 of [security.md](security.md)). Behind a tunnel such as Cloudflare Tunnel every user arrives from the tunnel's address, so all of them share that limit. When it is reached, the page retries what it was reading by itself (three tries in all, a second or two apart); a save or any other change is not retried. If the request is still refused, the page says **Too many requests**: wait a few seconds, then reload the page or try again. This is different from the sign-in lock after wrong passwords, which says how long to wait.
