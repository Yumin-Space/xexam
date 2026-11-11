# -*- coding: utf-8 -*-
"""The made-up demo question banks used by the README / docs screenshots (six-column layout).

Columns: A English question | B Chinese question | C Japanese question | D English answer | E Chinese answer |
F Japanese answer. Everything here is invented for the screenshots; nothing comes from a real exam.

  General knowledge.xlsx      sheets "Everyday" (5 questions) and "Numbers" (2)
  Kitchen basics.xlsx         sheets "Cooking" (3, one of them a hotspot) and "Safety" (2)
  Kitchen basics (v2).xlsx    the update shown in the upload preview: one new question, one updated, one retired, and
                              one Japanese answer that differs from the English one (so the preview shows its warning)

build(folder) writes the three files into `folder` (a temporary folder: .xlsx files are never committed) and returns
their paths. Needs openpyxl.
"""
import os

SP = '　'   # ideographic space between a hotspot sub-item and its choices


def ans(key_en, why_en, aid_en, key_zh, why_zh, aid_zh, key_ja, why_ja, aid_ja):
    return (
        '【Answer】%s\n\n【Explanation】%s\n\n【Memory aid】%s' % (key_en, why_en, aid_en),
        '【答案】%s\n\n【解析】%s\n\n【记忆口诀】%s' % (key_zh, why_zh, aid_zh),
        '【正解】%s\n\n【解説】%s\n\n【覚え方】%s' % (key_ja, why_ja, aid_ja),
    )


def q(en, zh, ja, answers):
    return (en, zh, ja) + answers


EVERYDAY = [
    q('January is the first month of the year. Which month is the first month of the year?\n\nA. March\nB. January\nC. May\nD. December',
      '一月是一年中的第一个月。一年中的第一个月是几月？\n\nA. 三月\nB. 一月\nC. 五月\nD. 十二月',
      '1月は1年の最初の月です。1年の最初の月は何月ですか？\n\nA. 3月\nB. 1月\nC. 5月\nD. 12月',
      ans('B', 'The question itself says that January is the first month; the other three come later in the year.',
          'Read the question: it often tells you the answer.',
          'B', '题干已经直接说了“一月是一年中的第一个月”，其余三个月都在一年中更晚的时候。', '答案看题干，题干会告诉你。',
          'B', '問題文に「1月は1年の最初の月」と書いてあります。ほかの三つは1年の後のほうの月です。', '答えは問題文の中にあることも多い。')),
    q('Which two of the following are fruits? (Choose 2)\n\nA. Apple\nB. Carrot\nC. Banana\nD. Onion\nE. Potato',
      '下面哪两个是水果？（选2项）\n\nA. 苹果\nB. 胡萝卜\nC. 香蕉\nD. 洋葱\nE. 土豆',
      '次のうち、果物を2つ選んでください。（2つ選択してください）\n\nA. りんご\nB. にんじん\nC. バナナ\nD. たまねぎ\nE. じゃがいも',
      ans('A, C', 'Apples and bananas are fruits; carrots, onions and potatoes are vegetables.',
          'Sweet and grows on a tree or plant you can eat raw: probably a fruit.',
          'A、C', '苹果和香蕉是水果；胡萝卜、洋葱、土豆都是蔬菜。', '长在树上、能直接吃的甜东西，多半是水果。',
          'A、C', 'りんごとバナナは果物です。にんじん、たまねぎ、じゃがいもは野菜です。', '甘くてそのまま食べられるものは、たいてい果物。')),
    q('HOTSPOT\nFor each statement, select Yes if it is correct and No if it is not.\n\n'
      '1. The sun rises in the east.' + SP + 'Yes / No\n\n2. The moon is a star.' + SP + 'Yes / No\n\n'
      '3. Water boils at 100 degrees (at standard pressure).' + SP + 'Yes / No',
      '热点题\n对下面每句话，正确选“是”，错误选“否”。\n\n1. 太阳从东边升起。' + SP + '是 / 否\n\n2. 月亮是恒星。' + SP + '是 / 否\n\n'
      '3. 水在100度沸腾（标准大气压）。' + SP + '是 / 否',
      'ホットスポット\n次の文について、正しければ「はい」、間違っていれば「いいえ」を選択してください。\n\n'
      '1. 太陽は東から昇る。' + SP + 'はい / いいえ\n\n2. 月は恒星である。' + SP + 'はい / いいえ\n\n'
      '3. 水は100度で沸騰する（標準気圧）。' + SP + 'はい / いいえ',
      ans('1 Yes; 2 No; 3 Yes', 'The moon is a satellite, not a star. The sun rising in the east and water boiling at 100 degrees '
          'at standard pressure are both correct.', 'A star makes its own light; the moon only reflects sunlight.',
          '1 是；2 否；3 是', '月亮是卫星，不是恒星；太阳东升、水在标准大气压下100度沸腾都是对的。', '会发光的才是恒星，月亮只是反射太阳光。',
          '1 はい；2 いいえ；3 はい', '月は衛星で、恒星ではありません。太陽が東から昇ること、標準気圧で水が100度で沸騰することは正しいです。',
          '自分で光るのが恒星。月は太陽の光を反射しているだけ。')),
    q('What should you do when the traffic light is red?\n\nA. Go\nB. Stop\nC. Run faster\nD. Turn around',
      '红灯时应该怎么做？\n\nA. 前进\nB. 停下\nC. 跑快点\nD. 转身向后',
      '信号が赤のとき、どうしますか？\n\nA. 進む\nB. 止まる\nC. 速く走る\nD. 後ろを向く',
      ans('B', 'Red means stop, green means go.', 'Red: stop. Green: go.',
          'B', '红灯停，绿灯行。', '红停绿行。',
          'B', '赤は止まれ、青は進め。', '赤で止まって、青で進む。')),
    q('Which two of the following are among the four seasons of the year? (Choose 2)\n\nA. Rainy season\nB. Spring\nC. Dry season\nD. Summer\nE. New school term',
      '下面哪两项属于一年的四季？（选2项）\n\nA. 雨季\nB. 春\nC. 旱季\nD. 夏\nE. 新学期',
      '一年の四季に含まれるものを2つ選んでください。（2つ選択してください）\n\nA. 雨季\nB. 春\nC. 乾季\nD. 夏\nE. 新学期',
      ans('B, D', 'The four seasons are spring, summer, autumn and winter. The rainy season, the dry season and a new school term are not among them.',
          'Spring, summer, autumn, winter: that is all four.',
          'B、D', '四季是春夏秋冬。雨季、旱季、新学期不是四季里的“季节”。', '春夏秋冬才是四季。',
          'B、D', '四季は春・夏・秋・冬です。雨季、乾季、新学期は四季に含まれません。', '春夏秋冬で四季。')),
]

NUMBERS = [
    q('What is 1 + 1?\n\nA. 1\nB. 2\nC. 3\nD. 4',
      '1 + 1 等于几？\n\nA. 1\nB. 2\nC. 3\nD. 4',
      '1 + 1 はいくつですか？\n\nA. 1\nB. 2\nC. 3\nD. 4',
      ans('B', 'Basic addition.', 'One plus one is two.',
          'B', '基础加法。', '一加一等于二。',
          'B', '基本の足し算です。', '一たす一は二。')),
    q('How many days are there in a week?\n\nA. 5 days\nB. 6 days\nC. 7 days\nD. 8 days',
      '一周有几天？\n\nA. 5天\nB. 6天\nC. 7天\nD. 8天',
      '一週間は何日ありますか？\n\nA. 5日\nB. 6日\nC. 7日\nD. 8日',
      ans('C', 'A week has seven days.', 'Monday to Sunday: seven days.',
          'C', '一周七天。', '周一到周日，七天。',
          'C', '一週間は7日です。', '月曜から日曜まで、7日。')),
]

COOKING = [
    q('HOTSPOT\nWhich stage of making soup does each step belong to?\n\n'
      '1. Wash and cut the vegetables.' + SP + 'Preparation / Cooking / Serving\n\n'
      '2. Simmer the soup for 20 minutes.' + SP + 'Preparation / Cooking / Serving\n\n'
      '3. Pour the soup into bowls.' + SP + 'Preparation / Cooking / Serving',
      '热点题\n做汤的每个步骤属于哪个阶段？\n\n1. 清洗并切好蔬菜。' + SP + '准备 / 烹饪 / 上菜\n\n'
      '2. 把汤小火炖 20 分钟。' + SP + '准备 / 烹饪 / 上菜\n\n3. 把汤盛进碗里。' + SP + '准备 / 烹饪 / 上菜',
      'ホットスポット\nスープを作る各手順は、どの段階に当たりますか？\n\n1. 野菜を洗って切る。' + SP + '下ごしらえ / 調理 / 盛り付け\n\n'
      '2. スープを 20 分間煮込む。' + SP + '下ごしらえ / 調理 / 盛り付け\n\n3. スープを器に注ぐ。' + SP + '下ごしらえ / 調理 / 盛り付け',
      ans('1 Preparation; 2 Cooking; 3 Serving', 'First prepare the ingredients, then heat them, and finally serve.',
          'Wash and cut, heat, pour.',
          '1 准备；2 烹饪；3 上菜', '先准备食材，再加热烹饪，最后装盘上桌。', '洗切、加热、装碗。',
          '1 下ごしらえ；2 調理；3 盛り付け', 'まず材料を準備し、次に加熱し、最後に器に盛ります。', '洗って切る、煮る、盛る。')),
    q('Which tool is best for cutting bread?\n\nA. Spoon\nB. Bread knife\nC. Whisk\nD. Ladle',
      '切面包最好用哪种工具？\n\nA. 勺子\nB. 面包刀\nC. 打蛋器\nD. 汤勺',
      'パンを切るのに最も適した道具はどれですか？\n\nA. スプーン\nB. パン切りナイフ\nC. 泡立て器\nD. おたま',
      ans('B', 'A bread knife has a serrated edge, so it cuts bread without squashing it.', 'Serrated knife for soft bread.',
          'B', '面包刀有锯齿，切面包不会压扁。', '锯齿刀切软面包。',
          'B', 'パン切りナイフは刃がギザギザなので、パンをつぶさずに切れます。', 'やわらかいパンはギザギザの刃で。')),
    q('Which two ingredients are usually in a pancake batter? (Choose 2)\n\nA. Flour\nB. Rice\nC. Eggs\nD. Seaweed',
      '松饼面糊里通常有哪两种材料？（选2项）\n\nA. 面粉\nB. 米饭\nC. 鸡蛋\nD. 海苔',
      'パンケーキの生地に普通入っている材料を2つ選んでください。（2つ選択してください）\n\nA. 小麦粉\nB. ご飯\nC. 卵\nD. 海苔',
      ans('A, C', 'Flour, eggs and milk are the basic ingredients of pancake batter.', 'Flour plus eggs makes a pancake.',
          'A、C', '面粉、鸡蛋、牛奶是松饼面糊的基本材料。', '粉加蛋，摊成饼。',
          'A、C', '小麦粉、卵、牛乳がパンケーキ生地の基本の材料です。', '粉と卵でパンケーキ。')),
]

SAFETY = [
    q('What should you do first if a pan of oil catches fire?\n\nA. Pour water on it\nB. Cover it with a lid and turn off the heat\nC. Carry it outside\nD. Blow on it',
      '油锅起火时，首先应该怎么做？\n\nA. 浇水\nB. 盖上锅盖并关火\nC. 端到室外\nD. 用嘴吹',
      '油の入った鍋から火が出たら、まず何をしますか？\n\nA. 水をかける\nB. ふたをして火を止める\nC. 外に運ぶ\nD. 息を吹きかける',
      ans('B', 'A lid cuts off the air and turning off the heat stops the fire growing; water makes an oil fire spread.',
          'Never water on burning oil: lid on, heat off.',
          'B', '盖上锅盖隔绝空气并关火；油锅浇水会让火势扩大。', '油火不浇水，盖盖再关火。',
          'B', 'ふたで空気を遮り、火を止めます。油の火に水をかけると燃え広がります。', '油の火に水はだめ。ふたをして火を止める。')),
    q('Where should raw chicken be stored in the fridge?\n\nA. On the top shelf\nB. On the bottom shelf\nC. In the door\nD. Next to the salad',
      '生鸡肉应该放在冰箱的哪里？\n\nA. 最上层\nB. 最下层\nC. 冰箱门上\nD. 沙拉旁边',
      '生の鶏肉は冷蔵庫のどこに保存すべきですか？\n\nA. 一番上の棚\nB. 一番下の棚\nC. ドアポケット\nD. サラダの隣',
      ans('B', 'On the bottom shelf its juices cannot drip onto other food.', 'Raw meat goes at the bottom.',
          'B', '放在最下层，汁水不会滴到其他食物上。', '生肉放底层。',
          'B', '一番下の棚なら、汁がほかの食品に垂れません。', '生の肉はいちばん下。')),
]

# New in v2. Its Japanese answer (column F) deliberately says C instead of B: the upload preview then shows the warning
# that the Japanese answer differs from the English one (the English answer is the one imported).
NEW_SAFETY = q('How long should you wash your hands before cooking?\n\nA. 2 seconds\nB. About 20 seconds\nC. 5 minutes\nD. No need',
               '做饭前应该洗手多长时间？\n\nA. 2 秒\nB. 大约 20 秒\nC. 5 分钟\nD. 不用洗',
               '料理の前は、どのくらい手を洗うべきですか？\n\nA. 2 秒\nB. 約 20 秒\nC. 5 分\nD. 洗わなくてよい',
               ans('B', 'Scrubbing with soap for about 20 seconds gets your hands clean.', 'Wash for twenty seconds.',
                   'B', '用肥皂搓洗大约 20 秒才能洗干净。', '洗手二十秒。',
                   'C', '石けんで約 20 秒こすり洗いすると、きれいになります。', '手洗いは 20 秒。'))

GENERAL = 'General knowledge.xlsx'
KITCHEN = 'Kitchen basics.xlsx'
KITCHEN_V2 = 'Kitchen basics (v2).xlsx'


def _save(folder, name, sheets):
    import openpyxl  # imported here so that the other modules work without it

    wb = openpyxl.Workbook()
    wb.remove(wb.active)
    for title, rows in sheets:
        ws = wb.create_sheet(title)
        for r in rows:
            ws.append(list(r))
    path = os.path.join(folder, name)
    wb.save(path)
    return path


def build(folder):
    """Writes the three demo workbooks into `folder`; returns {file name: path}."""
    os.makedirs(folder, exist_ok=True)
    # v2: question 2 of Cooking gets a longer explanation in all three languages (updated), question 2 of Safety is gone
    # (retired), and one Safety question is new
    cook2 = list(COOKING)
    c = list(cook2[1])
    c[3] = c[3].replace('without squashing it.', 'without squashing it; keep the blade sharp.')
    c[4] = c[4].replace('切面包不会压扁。', '切面包不会压扁；刀刃要保持锋利。')
    c[5] = c[5].replace('つぶさずに切れます。', 'つぶさずに切れます。刃は鋭く保ちましょう。')
    cook2[1] = tuple(c)
    return {
        GENERAL: _save(folder, GENERAL, [('Everyday', EVERYDAY), ('Numbers', NUMBERS)]),
        KITCHEN: _save(folder, KITCHEN, [('Cooking', COOKING), ('Safety', SAFETY)]),
        KITCHEN_V2: _save(folder, KITCHEN_V2, [('Cooking', cook2), ('Safety', [SAFETY[0], NEW_SAFETY])]),
    }


if __name__ == '__main__':
    import sys

    if len(sys.argv) != 2:
        sys.exit('usage: python tools/screenshots/demo_banks.py <output folder>')
    for name, path in build(sys.argv[1]).items():
        print('written', path)
