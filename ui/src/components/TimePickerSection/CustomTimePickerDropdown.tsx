import { css } from "@emotion/react";
import styled from "@emotion/styled";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

//
// region Styles
//

const ACCENT = "#9554d8";

const Dropdown = styled.div`
  position: absolute;
  top: calc(100% + 8px);
  left: 0;
  z-index: 1000;
  border-radius: var(--border-radius-lg);
  border: 1px solid var(--color-border-secondary);
  background: var(--color-bg-container);
  box-shadow: 0 10px 15px -3px rgba(0, 0, 0, 0.3), 0 4px 6px -2px rgba(0, 0, 0, 0.2);
  overflow: hidden;
  width: 320px;
`;

const CalendarContainer = styled.div`
  padding: var(--spacing-md);
`;

const CalendarHeader = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: var(--spacing-sm);
`;

const NavButton = styled.button`
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: 1px solid var(--color-border-secondary);
  border-radius: 50%;
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
  transition: all 0.2s ease;

  &:hover {
    background: var(--color-bg-spotlight);
    color: var(--color-text-base);
  }
`;

const MonthYear = styled.div`
  font-size: 13px;
  font-weight: 500;
  color: var(--color-text-base);
  text-align: center;
  flex: 1;
`;

const WeekDays = styled.div`
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  margin-bottom: var(--spacing-xs);
`;

const WeekDay = styled.div`
  font-size: 11px;
  font-weight: 500;
  color: var(--color-text-secondary);
  text-align: center;
  padding: var(--spacing-xs);
  text-transform: uppercase;
`;

const CalendarGrid = styled.div`
  display: grid;
  grid-template-columns: repeat(7, 1fr);
`;

const CalendarDay = styled.button<{
  isCurrentMonth: boolean;
  isSelected: boolean;
  isInRange: boolean;
  isToday: boolean;
  isRangeStart: boolean;
  isRangeEnd: boolean;
  isFutureDate: boolean;
}>`
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 34px;
  border: none;
  border-radius: 0;
  margin: 0;
  padding: 0;
  font-size: 12px;
  cursor: pointer;
  transition: all 0.15s ease;
  background: transparent;

  ${({ isCurrentMonth, isFutureDate }) =>
    !isCurrentMonth || isFutureDate
      ? css`
          color: var(--color-text-secondary);
          opacity: 0.4;
          cursor: not-allowed;
        `
      : css`
          color: var(--color-text-base);
          &:hover {
            background: var(--color-bg-spotlight);
          }
        `}

  ${({ isToday, isCurrentMonth }) =>
    isToday &&
    isCurrentMonth &&
    css`
      color: ${ACCENT};
      font-weight: 600;
    `}

  ${({ isInRange }) =>
    isInRange &&
    css`
      background: rgba(149, 84, 216, 0.15) !important;
      color: var(--color-text-base) !important;
      border-radius: 0 !important;
    `}

  ${({ isRangeStart, isRangeEnd }) =>
    (isRangeStart || isRangeEnd) &&
    css`
      background: ${ACCENT} !important;
      color: #fff !important;
    `}

  ${({ isRangeStart, isRangeEnd }) =>
    isRangeStart &&
    !isRangeEnd &&
    css`
      border-radius: var(--border-radius-sm) 0 0 var(--border-radius-sm) !important;
    `}

  ${({ isRangeEnd, isRangeStart }) =>
    isRangeEnd &&
    !isRangeStart &&
    css`
      border-radius: 0 var(--border-radius-sm) var(--border-radius-sm) 0 !important;
    `}

  ${({ isRangeStart, isRangeEnd }) =>
    isRangeStart &&
    isRangeEnd &&
    css`
      border-radius: var(--border-radius-sm) !important;
    `}
`;

const TimeContainer = styled.div`
  border-top: 1px solid var(--color-border-secondary);
  padding: var(--spacing-md);
  display: flex;
  gap: var(--spacing-md);
`;

const TimeSection = styled.div`
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
`;

const TimeLabel = styled.label`
  font-size: 11px;
  font-weight: 500;
  color: var(--color-text-secondary);
  text-transform: uppercase;
`;

const TimeInput = styled.input`
  padding: var(--spacing-xs) var(--spacing-sm);
  border: 1px solid var(--color-border-secondary);
  border-radius: var(--border-radius-md);
  background: var(--color-bg-spotlight);
  color: var(--color-text-base);
  font-size: var(--font-size-sm);

  &:focus {
    outline: none;
    border-color: ${ACCENT};
  }

  &::-webkit-calendar-picker-indicator {
    filter: invert(1);
    opacity: 0.6;
    cursor: pointer;
  }
`;

const ButtonContainer = styled.div`
  border-top: 1px solid var(--color-border-secondary);
  padding: var(--spacing-sm) var(--spacing-md);
  display: flex;
  gap: var(--spacing-xs);
  justify-content: flex-end;
`;

const ApplyButton = styled.button<{ disabled?: boolean }>`
  padding: 4px 12px;
  border: none;
  border-radius: var(--border-radius-sm);
  background: ${ACCENT};
  color: #fff;
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s ease;

  &:hover:not(:disabled) {
    opacity: 0.85;
  }

  &:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
`;

const CancelButton = styled.button`
  padding: 4px 12px;
  border: 1px solid var(--color-border-secondary);
  border-radius: var(--border-radius-sm);
  background: transparent;
  color: var(--color-text-base);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s ease;

  &:hover {
    background: var(--color-bg-spotlight);
  }
`;

//
// region Component
//

interface CustomTimePickerDropdownProps {
  isOpen: boolean;
  onClose: () => void;
  onApply: (start: Date, end: Date) => void;
  savedStart?: Date;
  savedEnd?: Date;
}

export const CustomTimePickerDropdown = ({
  isOpen,
  onClose,
  onApply,
  savedStart,
  savedEnd,
}: CustomTimePickerDropdownProps) => {
  const [startDate, setStartDate] = useState<Date | undefined>(savedStart);
  const [endDate, setEndDate] = useState<Date | undefined>(savedEnd);
  const [currentMonth, setCurrentMonth] = useState(savedStart ?? new Date());
  const [isSelectingEnd, setIsSelectingEnd] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  const handleClickOutside = useCallback(
    (event: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        onClose();
      }
    },
    [onClose]
  );

  const getDefaultStart = () => {
    const d = new Date();
    d.setDate(d.getDate() - 1);
    d.setHours(0, 0, 0, 0);
    return d;
  };

  const getDefaultEnd = () => {
    const d = new Date();
    d.setHours(23, 59, 59, 999);
    return d;
  };

  useEffect(() => {
    if (isOpen) {
      const start = savedStart ?? getDefaultStart();
      const end = savedEnd ?? getDefaultEnd();
      setStartDate(start);
      setEndDate(end);
      setCurrentMonth(start);
      document.addEventListener("mousedown", handleClickOutside);
      return () => document.removeEventListener("mousedown", handleClickOutside);
    }
  }, [isOpen, savedStart, savedEnd, handleClickOutside]);

  if (!isOpen) return null;

  const isFutureDate = (date: Date) => {
    const today = new Date();
    today.setHours(23, 59, 59, 999);
    return date > today;
  };

  const handleDateClick = (clicked: Date) => {
    if (isFutureDate(clicked) || !clicked) return;

    if (!startDate || (!isSelectingEnd && startDate && endDate)) {
      const d = new Date(clicked);
      d.setHours(0, 0, 0, 0);
      setStartDate(d);
      setEndDate(undefined);
      setIsSelectingEnd(true);
    } else if (isSelectingEnd && startDate) {
      const d = new Date(clicked);
      d.setHours(23, 59, 59, 999);
      if (d < startDate) {
        const newStart = new Date(clicked);
        newStart.setHours(0, 0, 0, 0);
        const newEnd = new Date(startDate);
        newEnd.setHours(23, 59, 59, 999);
        setStartDate(newStart);
        setEndDate(newEnd);
      } else {
        setEndDate(d);
      }
      setIsSelectingEnd(false);
    }
  };

  const handleTimeChange = (isStart: boolean, value: string) => {
    const [hours, minutes] = value.split(":").map(Number);
    if (isStart && startDate) {
      const d = new Date(startDate);
      d.setHours(hours, minutes, 0, 0);
      setStartDate(d);
    } else if (!isStart && endDate) {
      const d = new Date(endDate);
      d.setHours(hours, minutes, 59, 999);
      setEndDate(d);
    }
  };

  const navigateMonth = (direction: "prev" | "next") => {
    setCurrentMonth((prev) => {
      const d = new Date(prev);
      d.setMonth(prev.getMonth() + (direction === "next" ? 1 : -1));
      return d;
    });
  };

  const isDateInRange = (date: Date) => {
    if (!startDate || !endDate) return false;
    return date >= startDate && date <= endDate;
  };

  const isDateSelected = (date: Date) => {
    const s = date.toDateString();
    return startDate?.toDateString() === s || endDate?.toDateString() === s;
  };

  const formatTime = (date?: Date) =>
    date ? date.toTimeString().slice(0, 5) : "";

  const renderDays = () => {
    const year = currentMonth.getFullYear();
    const month = currentMonth.getMonth();
    const firstDayOfMonth = new Date(year, month, 1);
    const lastDayOfMonth = new Date(year, month + 1, 0);
    const firstDayOfWeek = firstDayOfMonth.getDay();
    const days: React.ReactNode[] = [];

    for (let i = firstDayOfWeek - 1; i >= 0; i--) {
      const d = new Date(year, month, -i);
      days.push(
        <CalendarDay
          key={`prev-${i}`}
          isCurrentMonth={false}
          isSelected={isDateSelected(d)}
          isInRange={isDateInRange(d)}
          isToday={false}
          isRangeStart={Boolean(startDate && d.toDateString() === startDate.toDateString())}
          isRangeEnd={Boolean(endDate && d.toDateString() === endDate.toDateString())}
          isFutureDate={isFutureDate(d)}
          onClick={() => handleDateClick(d)}
        >
          {d.getDate()}
        </CalendarDay>
      );
    }

    for (let day = 1; day <= lastDayOfMonth.getDate(); day++) {
      const d = new Date(year, month, day);
      days.push(
        <CalendarDay
          key={day}
          isCurrentMonth={true}
          isSelected={isDateSelected(d)}
          isInRange={isDateInRange(d)}
          isToday={d.toDateString() === new Date().toDateString()}
          isRangeStart={Boolean(startDate && d.toDateString() === startDate.toDateString())}
          isRangeEnd={Boolean(endDate && d.toDateString() === endDate.toDateString())}
          isFutureDate={isFutureDate(d)}
          onClick={() => handleDateClick(d)}
        >
          {day}
        </CalendarDay>
      );
    }

    const remaining = 42 - days.length;
    for (let day = 1; day <= remaining; day++) {
      const d = new Date(year, month + 1, day);
      days.push(
        <CalendarDay
          key={`next-${day}`}
          isCurrentMonth={false}
          isSelected={isDateSelected(d)}
          isInRange={isDateInRange(d)}
          isToday={false}
          isRangeStart={Boolean(startDate && d.toDateString() === startDate.toDateString())}
          isRangeEnd={Boolean(endDate && d.toDateString() === endDate.toDateString())}
          isFutureDate={isFutureDate(d)}
          onClick={() => handleDateClick(d)}
        >
          {day}
        </CalendarDay>
      );
    }

    return days;
  };

  return (
    <Dropdown ref={containerRef}>
      <CalendarContainer>
        <CalendarHeader>
          <NavButton onClick={() => navigateMonth("prev")}>
            <ChevronLeft size={14} />
          </NavButton>
          <MonthYear>
            {currentMonth.toLocaleDateString("en-US", { month: "long", year: "numeric" })}
          </MonthYear>
          <NavButton onClick={() => navigateMonth("next")}>
            <ChevronRight size={14} />
          </NavButton>
        </CalendarHeader>

        <WeekDays>
          {["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].map((d) => (
            <WeekDay key={d}>{d}</WeekDay>
          ))}
        </WeekDays>

        <CalendarGrid>{renderDays()}</CalendarGrid>
      </CalendarContainer>

      <TimeContainer>
        <TimeSection>
          <TimeLabel>Start Time</TimeLabel>
          <TimeInput
            type="time"
            value={formatTime(startDate)}
            disabled={!startDate}
            onChange={(e) => handleTimeChange(true, e.target.value)}
          />
        </TimeSection>
        <TimeSection>
          <TimeLabel>End Time</TimeLabel>
          <TimeInput
            type="time"
            value={formatTime(endDate)}
            disabled={!endDate}
            onChange={(e) => handleTimeChange(false, e.target.value)}
          />
        </TimeSection>
      </TimeContainer>

      <ButtonContainer>
        <CancelButton onClick={onClose}>Cancel</CancelButton>
        <ApplyButton
          disabled={!startDate || !endDate}
          onClick={() => startDate && endDate && onApply(startDate, endDate)}
        >
          Apply
        </ApplyButton>
      </ButtonContainer>
    </Dropdown>
  );
};
