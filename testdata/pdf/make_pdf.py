"""Builds the PDFs of the pdf tests (reportlab + pypdf):

- three.pdf: 3 pages with "Page 1".."Page 3" (A4 portrait, A4 landscape, A5),
  the third turned with /Rotate 90, and an outline entry to each page.
- secret.pdf: one page, protected by the user password "secret".
"""
from reportlab.lib.pagesizes import A4, A5, landscape
from reportlab.pdfgen import canvas
from pypdf import PdfReader, PdfWriter

c = canvas.Canvas("three_src.pdf", pagesize=A4)
for i, size in enumerate([A4, landscape(A4), A5], 1):
    c.setPageSize(size)
    c.setFont("Helvetica", 24)
    c.drawString(72, size[1] - 100, f"Page {i}")
    c.bookmarkPage(f"p{i}")
    c.addOutlineEntry(f"Page {i}", f"p{i}")
    c.showPage()
c.save()

r = PdfReader("three_src.pdf")
w = PdfWriter(clone_from=r)
w.pages[2].rotate(90)
with open("three.pdf", "wb") as f:
    w.write(f)

c = canvas.Canvas("secret_src.pdf", pagesize=A4)
c.drawString(72, 720, "Secret")
c.showPage()
c.save()
w = PdfWriter(clone_from=PdfReader("secret_src.pdf"))
w.encrypt("secret", "owner", algorithm="AES-128")
with open("secret.pdf", "wb") as f:
    w.write(f)

import os
os.remove("three_src.pdf")
os.remove("secret_src.pdf")
