import { definePreset } from '@primeuix/themes'
import Aura from '@primeuix/themes/aura'

const controlBackground = 'rgb(2 6 23 / 42%)'
const controlBorder = 'rgb(148 163 184 / 18%)'
const controlFocus = 'rgb(96 165 250 / 80%)'
const focusShadow = '0 0 0 1px rgb(59 130 246 / 24%), 0 0 0 4px rgb(59 130 246 / 10%)'

const controlRoot = {
  background: controlBackground,
  borderColor: controlBorder,
  hoverBorderColor: 'rgb(96 165 250 / 38%)',
  focusBorderColor: controlFocus,
  color: '#e2e8f0',
  placeholderColor: '#64748b',
  shadow: 'inset 0 1px 0 rgb(226 232 240 / 4%)',
  focusRing: {
    width: '0',
    style: 'none',
    color: 'transparent',
    offset: '0',
    shadow: focusShadow,
  },
}

const NvrPrimePreset = definePreset(Aura, {
  components: {
    datatable: {
      root: {
        borderColor: 'rgb(148 163 184 / 10%)',
      },
      header: {
        background: 'rgb(30 41 59 / 40%)',
        borderColor: 'rgb(148 163 184 / 10%)',
        color: '#e2e8f0',
      },
      headerCell: {
        background: 'linear-gradient(180deg, rgb(30 41 59 / 88%), rgb(15 23 42 / 78%))',
        hoverBackground: 'rgb(30 41 59 / 86%)',
        selectedBackground: 'rgb(30 41 59 / 92%)',
        borderColor: 'rgb(148 163 184 / 10%)',
        color: '#cbd5e1',
        hoverColor: '#e2e8f0',
        selectedColor: '#bfdbfe',
        padding: '0.75rem 1rem',
        sm: {
          padding: '0.75rem 1rem',
        },
      },
      columnTitle: {
        fontWeight: '600',
      },
      row: {
        background: 'rgb(15 23 42 / 28%)',
        stripedBackground: 'rgb(18 28 48 / 34%)',
        hoverBackground: 'rgb(22 34 57 / 40%)',
        selectedBackground: 'rgb(59 130 246 / 18%)',
        color: '#e2e8f0',
        hoverColor: '#f8fafc',
        selectedColor: '#bfdbfe',
      },
      bodyCell: {
        borderColor: 'rgb(148 163 184 / 8%)',
        padding: '0.75rem 1rem',
        selectedBorderColor: 'rgb(96 165 250 / 22%)',
        sm: {
          padding: '0.75rem 1rem',
        },
      },
      loadingIcon: {
        size: '2rem',
      },
      css: `
        .p-datatable {
          background: transparent;
          color: #e2e8f0;
        }

        .p-datatable .p-datatable-table-container {
          background: rgb(15 23 42 / 40%);
          backdrop-filter: blur(12px);
          border: 1px solid rgb(148 163 184 / 10%);
          border-radius: 0.75rem;
          box-shadow: 0 4px 12px rgb(0 0 0 / 20%);
          overflow: hidden;
        }

        .p-datatable .p-datatable-thead > tr > th {
          font-size: 0.8125rem;
          letter-spacing: 0.02em;
          box-shadow: inset 0 -1px 0 rgb(148 163 184 / 8%);
        }

        .p-datatable .p-datatable-tbody > tr > td {
          font-size: 0.8125rem;
        }

        .p-datatable.p-datatable-striped .p-datatable-tbody > tr.p-row-even {
          background: rgb(15 23 42 / 28%);
        }

        .p-datatable.p-datatable-striped .p-datatable-tbody > tr.p-row-odd {
          background: rgb(18 28 48 / 34%);
        }

        .p-datatable.p-datatable-striped.p-datatable-hoverable .p-datatable-tbody > tr.p-row-even:not(.p-datatable-row-selected):hover,
        .p-datatable.p-datatable-striped.p-datatable-hoverable .p-datatable-tbody > tr.p-row-odd:not(.p-datatable-row-selected):hover {
          background: rgb(22 34 57 / 40%);
        }

        .p-datatable .p-datatable-tbody > tr > td,
        .p-datatable .p-datatable-tbody > tr > td.p-datatable-frozen-column {
          background: inherit;
        }

        .p-datatable .p-datatable-emptymessage > td {
          text-align: center;
          padding: 2rem;
        }

        .p-datatable-loading-icon {
          color: #3b82f6;
        }
      `,
    },
    dialog: {
      root: {
        background:
          'radial-gradient(circle at 12% 0%, rgb(59 130 246 / 14%), transparent 28rem), linear-gradient(145deg, rgb(15 23 42 / 98%), rgb(30 41 59 / 94%))',
        borderColor: 'rgb(96 165 250 / 20%)',
        color: '#cbd5e1',
        borderRadius: '1rem',
        shadow: '0 28px 80px rgb(2 6 23 / 68%), inset 0 1px 0 rgb(226 232 240 / 8%)',
      },
      header: {
        padding: '1.25rem 1.35rem 1rem',
        gap: '0.5rem',
      },
      title: {
        fontSize: '1rem',
        fontWeight: '700',
      },
      content: {
        padding: '1.35rem',
      },
      footer: {
        padding: '0 1.35rem 1.35rem',
        gap: '0.5rem',
      },
      css: `
        .p-dialog {
          overflow: hidden;
          backdrop-filter: blur(12px);
        }

        .p-dialog .p-dialog-header {
          position: relative;
          background: linear-gradient(180deg, rgb(30 41 59 / 62%), rgb(15 23 42 / 0%));
          border-bottom: 1px solid rgb(148 163 184 / 12%);
          color: #e2e8f0;
        }

        .p-dialog .p-dialog-header::after {
          content: "";
          position: absolute;
          left: 1.35rem;
          right: 1.35rem;
          bottom: -1px;
          height: 1px;
          background: linear-gradient(90deg, rgb(59 130 246 / 72%), transparent 68%);
        }

        .p-dialog .p-dialog-title {
          color: #e2e8f0;
          letter-spacing: -0.01em;
        }

        .p-dialog .p-dialog-content {
          background: transparent;
          color: #cbd5e1;
        }

        .p-dialog .p-dialog-header-icon,
        .p-dialog .p-dialog-header-close {
          width: 2rem;
          height: 2rem;
          color: #94a3b8;
          border-radius: 0.55rem;
        }

        .p-dialog .p-dialog-header-icon:hover,
        .p-dialog .p-dialog-header-close:hover {
          color: #e2e8f0;
          background: rgb(148 163 184 / 12%);
        }
      `,
    },
    inputtext: {
      root: controlRoot,
    },
    textarea: {
      root: controlRoot,
    },
    select: {
      root: controlRoot,
      dropdown: {
        color: '#cbd5e1',
      },
      overlay: {
        background: 'rgb(15 23 42 / 98%)',
        borderColor: 'rgb(148 163 184 / 16%)',
        borderRadius: '0.75rem',
        color: '#e2e8f0',
        shadow: '0 18px 54px rgb(2 6 23 / 62%)',
      },
      option: {
        color: '#cbd5e1',
        focusColor: '#e2e8f0',
        focusBackground: 'rgb(148 163 184 / 10%)',
        selectedColor: '#bfdbfe',
        selectedBackground: 'rgb(59 130 246 / 18%)',
        selectedFocusColor: '#bfdbfe',
        selectedFocusBackground: 'rgb(59 130 246 / 22%)',
      },
      css: `
        .p-select-overlay {
          overflow: hidden;
          backdrop-filter: blur(14px);
        }

        .p-select-list-container {
          background: transparent;
        }
      `,
    },
    tabs: {
      tablist: {
        background: 'rgb(30 41 59 / 40%)',
        borderColor: 'rgb(148 163 184 / 10%)',
      },
      tab: {
        background: 'transparent',
        hoverBackground: 'rgb(148 163 184 / 10%)',
        activeBackground: 'rgb(59 130 246 / 10%)',
        borderColor: 'transparent',
        activeBorderColor: '#3b82f6',
        color: '#94a3b8',
        hoverColor: '#cbd5e1',
        activeColor: '#3b82f6',
      },
      tabpanel: {
        background: 'transparent',
        color: '#e2e8f0',
      },
      activeBar: {
        background: '#3b82f6',
      },
      css: `
        .p-tablist-content,
        .p-tablist-tab-list {
          background: transparent;
        }

        .p-tablist-active-bar {
          box-shadow: 0 0 12px rgb(59 130 246 / 50%);
        }
      `,
    },
    tag: {
      root: {
        fontSize: '0.6875rem',
        fontWeight: '700',
        padding: '0.25rem 0.5rem',
        borderRadius: '999px',
        roundedBorderRadius: '999px',
      },
      primary: {
        background: 'linear-gradient(180deg, rgb(59 130 246 / 24%), rgb(37 99 235 / 12%))',
        color: '#bfdbfe',
      },
      info: {
        background: 'linear-gradient(180deg, rgb(59 130 246 / 24%), rgb(37 99 235 / 12%))',
        color: '#bfdbfe',
      },
      secondary: {
        background: 'linear-gradient(180deg, rgb(71 85 105 / 34%), rgb(30 41 59 / 30%))',
        color: '#cbd5e1',
      },
      contrast: {
        background: 'linear-gradient(180deg, rgb(15 23 42 / 88%), rgb(2 6 23 / 64%))',
        color: '#e2e8f0',
      },
      success: {
        background: 'linear-gradient(180deg, rgb(16 185 129 / 24%), rgb(5 150 105 / 12%))',
        color: '#bbf7d0',
      },
      warn: {
        background: 'linear-gradient(180deg, rgb(245 158 11 / 24%), rgb(217 119 6 / 12%))',
        color: '#fde68a',
      },
      danger: {
        background: 'linear-gradient(180deg, rgb(239 68 68 / 24%), rgb(185 28 28 / 12%))',
        color: '#fecaca',
      },
      css: `
        .p-tag {
          border: 1px solid rgb(148 163 184 / 12%);
          box-shadow:
            inset 0 1px 0 rgb(226 232 240 / 6%),
            0 1px 2px rgb(2 6 23 / 24%);
          letter-spacing: 0.02em;
        }

        .p-tag-info {
          border-color: rgb(96 165 250 / 28%);
        }

        .p-tag-secondary {
          border-color: rgb(148 163 184 / 16%);
        }

        .p-tag-contrast {
          border-color: rgb(226 232 240 / 18%);
        }

        .p-tag-success {
          border-color: rgb(52 211 153 / 28%);
        }

        .p-tag-warn {
          border-color: rgb(251 191 36 / 30%);
        }

        .p-tag-danger {
          border-color: rgb(248 113 113 / 30%);
        }

        .p-tag-label {
          line-height: 1.2;
        }
      `,
    },
    paginator: {
      root: {
        background: 'rgb(30 41 59 / 40%)',
        color: '#cbd5e1',
      },
      navButton: {
        background: 'transparent',
        hoverBackground: 'rgb(148 163 184 / 10%)',
        selectedBackground: 'rgb(59 130 246 / 15%)',
        color: '#94a3b8',
        hoverColor: '#cbd5e1',
        selectedColor: '#3b82f6',
      },
      css: `
        .p-paginator {
          border-color: rgb(148 163 184 / 10%);
        }

        .p-paginator .p-paginator-page.p-paginator-page-active {
          border-color: #3b82f6;
        }
      `,
    },
  },
})

export default NvrPrimePreset
