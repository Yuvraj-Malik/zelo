import { forwardRef } from "react";

interface Props {
  value: string;
  onChange: (v: string) => void;
}

const SearchBar = forwardRef<HTMLInputElement, Props>(({ value, onChange }, ref) => (
  <div className="searchbar">
    <span className="icon">⌕</span>
    <input
      ref={ref}
      autoFocus
      spellCheck={false}
      placeholder="Zelo Search"
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  </div>
));

export default SearchBar;
