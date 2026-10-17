import { LocalizedText } from '../lib/LocalizedText'
import { interfaceTextProps, translateInterface } from '../lib/interfaceLocale'
export type DetailPosition = 'right' | 'left' | 'bottom' | 'top'

export const DETAIL_POSITION_OPTIONS: Array<{ value: DetailPosition; label: string }> = [
  { value: 'right', label: 'Right' },
  { value: 'left', label: 'Left' },
  { value: 'bottom', label: 'Bottom' },
  { value: 'top', label: 'Top' },
]

export function isDetailPosition(value: string | null): value is DetailPosition {
  return value === 'right' || value === 'left' || value === 'bottom' || value === 'top'
}

interface Props {
  position: DetailPosition
  onChange: (position: DetailPosition) => void
}

export function DetailDockControls({ position, onChange }: Props) {
  return (
    <label className="tng-detail-position" title={translateInterface("Choose where the details panel is docked")} {...interfaceTextProps("Choose where the details panel is docked")}>
      <span><LocalizedText source="Panel" /></span>
      <select
        aria-label={translateInterface("Details panel position")}
        value={position}
        onChange={event => {
          if (isDetailPosition(event.target.value)) onChange(event.target.value)
        }}
       {...interfaceTextProps("Details panel position")}>
        {DETAIL_POSITION_OPTIONS.map(option => (
          <option key={option.value} value={option.value}><LocalizedText source={option.label} /></option>
        ))}
      </select>
    </label>
  )
}
