"""Builds print.xlsx: page setups for the tests of the print layout
(src/api/xlsx_print.rs, book.toPdf). Run with openpyxl and Pillow:
python3 testdata/xlsx/make_print.py"""
import io, os
from openpyxl import Workbook
from openpyxl.chart import BarChart, Reference
from openpyxl.drawing.image import Image
from openpyxl.styles import Alignment, Border, Font, PatternFill, Side
from openpyxl.styles.colors import Color
from openpyxl.worksheet.pagebreak import Break
from PIL import Image as PILImage, ImageDraw

wb = Workbook()
ws = wb.active
ws.title = "一覧"
head = ["No", "日付", "支店", "担当", "商品", "数量", "単価", "金額", "備考"]
ws.append(head)
for i in range(1, 81):
    ws.append([i, f"2026/04/{i % 28 + 1:02d}", ["東京", "大阪", "名古屋"][i % 3], f"担当{i % 5}", f"商品{i % 9}", i % 13 + 1, 100 * (i % 7 + 1), None,
               "折り返して表示する長めの備考です" if i % 10 == 0 else None])
    ws.cell(i + 1, 8).value = f"=F{i + 1}*G{i + 1}"
thin = Side(style="thin", color="808080")
for row in ws.iter_rows(min_row=1, max_row=81, max_col=9):
    for c in row:
        c.border = Border(left=thin, right=thin, top=thin, bottom=thin)
for c in ws[1]:
    c.font = Font(bold=True, color="FFFFFF")
    # Theme color (accent1, lighter 40%) for the cells of the theme.
    c.fill = PatternFill("solid", fgColor=Color(theme=4, tint=-0.25))
    c.alignment = Alignment(horizontal="center")
for r in range(2, 82):
    ws.cell(r, 7).number_format = "#,##0"
    ws.cell(r, 8).number_format = "#,##0"
    ws.cell(r, 9).alignment = Alignment(wrap_text=True, vertical="top")
widths = {"A": 5, "B": 11, "C": 8, "D": 8, "E": 10, "F": 6, "G": 8, "H": 10, "I": 24}
for k, v in widths.items():
    ws.column_dimensions[k].width = v
ws.column_dimensions["J"].hidden = True
ws["K1"] = "縦書き"
ws["K1"].alignment = Alignment(text_rotation=255)
ws["L1"] = "45度"
ws["L1"].alignment = Alignment(text_rotation=45)
ws.row_dimensions[1].height = 40

ws.print_title_rows = "1:1"
ws.page_setup.orientation = "landscape"
ws.page_setup.paperSize = ws.PAPERSIZE_A4
ws.page_setup.fitToWidth = 1
ws.page_setup.fitToHeight = 0
ws.sheet_properties.pageSetUpPr.fitToPage = True
ws.print_options.horizontalCentered = True
ws.oddHeader.left.text = "&\"游ゴシック,太字\"売上一覧"
ws.oddHeader.right.text = "&A"
ws.oddFooter.center.text = "&P / &N ページ"
ws.row_breaks.append(Break(id=40))

ch = BarChart()
ch.title = "数量"
ch.add_data(Reference(ws, min_col=6, min_row=1, max_row=11), titles_from_data=True)
ch.set_categories(Reference(ws, min_col=1, min_row=2, max_row=11))
ch.width, ch.height = 12, 6
ws.add_chart(ch, "M3")

img = PILImage.new("RGB", (160, 80), "#FFF2CC")
d = ImageDraw.Draw(img)
d.rectangle([4, 4, 155, 75], outline="#C00000", width=4)
d.line([4, 4, 155, 75], fill="#C00000", width=3)
buf = io.BytesIO()
img.save(buf, "PNG")
buf.seek(0)
ws.add_image(Image(buf), "M20")

pa = wb.create_sheet("印刷範囲")
for r in range(1, 21):
    for c in range(1, 8):
        pa.cell(r, c).value = f"{chr(64 + c)}{r}"
pa.print_area = "B2:D10"
pa.page_setup.paperSize = 13  # B5 (JIS)

wb.create_sheet("空")
hidden = wb.create_sheet("非表示")
hidden["A1"] = "x"
hidden.sheet_state = "hidden"

wb.save(os.path.join(os.path.dirname(os.path.abspath(__file__)), "print.xlsx"))
